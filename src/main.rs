use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use personal_taoli::{
    account::{AccountData, load_account_data},
    archive::{
        ArchiveWriter, ArchivedEvent, DecisionEvent, HealthEvent, ShadowReport, default_gap_path,
        replay_archive,
    },
    config::ObserverConfig,
    instrument::{InstrumentSpec, validate_pair},
    local_book::{BookFeed, BookFeedStatus, BookState},
    market::unix_timestamp_ms,
    order::{OrderFactsSmokeReport, run_order_facts_smoke},
    paper::{PaperCoreSmokeReport, run_paper_core_smoke},
    scan::{
        FreshnessLimits, InstrumentPair, ScanInput, ScanReport, VenueFeeRates, VenueFees, scan_pair,
    },
    venues::{BinanceMarketData, BybitMarketData, MarketDataVenue},
};
use reqwest::Client;
use serde::Serialize;
use tokio::time::{MissedTickBehavior, interval, timeout};

#[derive(Debug, Parser)]
#[command(version, about = "Read-only arbitrage observer and PAPER safety core")]
struct Cli {
    #[arg(short, long, default_value = "config/observer.toml")]
    config: PathBuf,
    #[arg(long, conflicts_with_all = ["reconnect_smoke", "account_check", "replay"], help = "Wait for live books, archive one decision, then exit")]
    once: bool,
    #[arg(
        long,
        conflicts_with_all = ["once", "account_check", "replay"],
        help = "Force both live feeds to reconnect, verify recovery, then exit"
    )]
    reconnect_smoke: bool,
    #[arg(
        long,
        conflicts_with_all = ["once", "reconnect_smoke", "replay"],
        help = "Load and print capability, permission and account fee metadata, then exit"
    )]
    account_check: bool,
    #[arg(
        long,
        value_name = "PATH",
        help = "Override the configured append-only observation archive"
    )]
    archive: Option<PathBuf>,
    #[arg(
        long,
        value_name = "PATH",
        conflicts_with_all = ["once", "reconnect_smoke", "account_check", "paper_core_smoke", "order_facts_smoke"],
        help = "Replay an archive deterministically and print its shadow report"
    )]
    replay: Option<PathBuf>,
    #[arg(
        long,
        conflicts_with_all = ["once", "reconnect_smoke", "account_check", "replay", "paper_core_smoke", "order_facts_smoke"],
        help = "Suppress per-scan output in continuous observation mode"
    )]
    quiet: bool,
    #[arg(
        long,
        conflicts_with_all = ["once", "reconnect_smoke", "account_check", "replay", "archive", "quiet", "order_facts_smoke"],
        help = "Verify B-01 PAPER reservation, recovery and single-writer invariants"
    )]
    paper_core_smoke: bool,
    #[arg(
        long,
        conflicts_with_all = ["once", "reconnect_smoke", "account_check", "replay", "archive", "quiet", "paper_core_smoke"],
        help = "Verify B-02 order facts and UNKNOWN transitions against PostgreSQL without order calls"
    )]
    order_facts_smoke: bool,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    output: OutputFormat,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(path) = &cli.replay {
        let report = replay_archive(path, default_gap_path(path))?;
        print_shadow_report(&report, cli.output)?;
        return Ok(());
    }
    if cli.order_facts_smoke {
        let database_url = std::env::var("TAOLI_DATABASE_URL")
            .context("TAOLI_DATABASE_URL is required for --order-facts-smoke")?;
        let report = run_order_facts_smoke(&database_url).await?;
        print_order_facts_smoke(&report, cli.output)?;
        return Ok(());
    }
    if cli.paper_core_smoke {
        let database_url = std::env::var("TAOLI_DATABASE_URL")
            .context("TAOLI_DATABASE_URL is required for --paper-core-smoke")?;
        let report = run_paper_core_smoke(&database_url).await?;
        print_paper_core_smoke(&report, cli.output)?;
        return Ok(());
    }
    let config = ObserverConfig::load(&cli.config)?;
    let archive_path = cli
        .archive
        .clone()
        .unwrap_or_else(|| config.archive.path.clone());
    let gap_path = default_gap_path(&archive_path);
    let client = Client::builder()
        .timeout(config.http_timeout())
        .user_agent(concat!("personal-taoli/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to build HTTP client")?;
    let binance: Arc<dyn MarketDataVenue> = Arc::new(BinanceMarketData::new(
        client.clone(),
        &config.binance.base_url,
        &config.binance.websocket_url,
    )?);
    let bybit: Arc<dyn MarketDataVenue> = Arc::new(BybitMarketData::new(
        client.clone(),
        &config.bybit.base_url,
        &config.bybit.websocket_url,
    )?);
    let mut accounts = load_account_data(
        &client,
        &config.symbol,
        &config.binance,
        &config.bybit,
        config.auth_recv_window_ms,
        config.max_fee_age_ms,
    )
    .await;
    if cli.account_check {
        print_accounts(&accounts, cli.output)?;
        return Ok(());
    }
    let (binance_instrument, bybit_instrument) = tokio::try_join!(
        binance.load_instrument(&config.symbol),
        bybit.load_instrument(&config.symbol),
    )?;
    validate_pair(
        &binance_instrument,
        &bybit_instrument,
        &config.symbol,
        &config.base_asset,
        &config.quote_asset,
        config.quantity,
    )?;

    let stale_after = Duration::from_millis(config.max_snapshot_age_ms);
    let mut binance_feed = binance.subscribe_order_book(
        &config.symbol,
        config.orderbook_depth,
        stale_after,
        config.reconnect_delay(),
    )?;
    let mut bybit_feed = bybit.subscribe_order_book(
        &config.symbol,
        config.orderbook_depth,
        stale_after,
        config.reconnect_delay(),
    )?;
    wait_for_valid_pair(
        &mut binance_feed,
        &mut bybit_feed,
        config.stream_start_timeout(),
    )
    .await?;

    if cli.reconnect_smoke {
        run_reconnect_smoke(
            &mut binance_feed,
            &mut bybit_feed,
            config.stream_start_timeout(),
        )
        .await?;
        return Ok(());
    }

    if cli.once {
        let mut archive = ArchiveWriter::start(
            &archive_path,
            &gap_path,
            config.archive.queue_capacity,
            config.archive.raw_retention_days,
        )?;
        let event = scan_current(
            &config,
            &binance_feed.status(),
            &bybit_feed.status(),
            &binance_instrument,
            &bybit_instrument,
            &accounts,
        )?;
        print_report(&event.report, &accounts, cli.output)?;
        submit_archive(&mut archive, ArchivedEvent::Decision(Box::new(event)));
        if let Err(error) = archive.finish() {
            eprintln!("archive shutdown failed: {error:#}");
        }
        return Ok(());
    }

    eprintln!(
        "OBSERVE mode: symbol={} quantity={} {} (live public feeds, optional read-only account metadata, no orders)",
        config.symbol, config.quantity, config.base_asset
    );
    let mut archive = ArchiveWriter::start(
        &archive_path,
        &gap_path,
        config.archive.queue_capacity,
        config.archive.raw_retention_days,
    )?;
    eprintln!(
        "ARCHIVE path={} gaps={}",
        archive_path.display(),
        gap_path.display()
    );
    let mut ticker = interval(config.poll_interval());
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut account_ticker = interval(config.account_refresh_interval());
    account_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    account_ticker.tick().await;
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let binance_status = binance_feed.status();
                let bybit_status = bybit_feed.status();
                match scan_current(
                    &config,
                    &binance_status,
                    &bybit_status,
                    &binance_instrument,
                    &bybit_instrument,
                    &accounts,
                ) {
                    Ok(event) => {
                        if !cli.quiet {
                            print_report(&event.report, &accounts, cli.output)?;
                        }
                        submit_archive(&mut archive, ArchivedEvent::Decision(Box::new(event)));
                    }
                    Err(error) => {
                        eprintln!("scan skipped: {error:#}");
                        submit_archive(
                            &mut archive,
                            ArchivedEvent::Health(Box::new(HealthEvent {
                                observed_at_ms: unix_timestamp_ms().unwrap_or(u64::MAX),
                                feeds: [(&binance_status).into(), (&bybit_status).into()],
                                skip_reason: format!("{error:#}"),
                            })),
                        );
                    }
                }
            }
            _ = account_ticker.tick() => {
                accounts = load_account_data(
                    &client,
                    &config.symbol,
                    &config.binance,
                    &config.bybit,
                    config.auth_recv_window_ms,
                    config.max_fee_age_ms,
                ).await;
            }
            result = &mut shutdown => {
                result?;
                eprintln!("observer stopped; flushing archive");
                break;
            }
        }
    }
    if let Err(error) = archive.finish() {
        eprintln!("archive shutdown failed: {error:#}");
    }
    Ok(())
}

async fn shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .context("failed to listen for SIGTERM")?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result.context("failed to listen for Ctrl-C")?;
            }
            _ = terminate.recv() => {}
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("failed to listen for Ctrl-C")
    }
}

async fn wait_for_valid_pair(
    first: &mut BookFeed,
    second: &mut BookFeed,
    start_timeout: Duration,
) -> Result<()> {
    timeout(start_timeout, async {
        loop {
            let first_status = first.status();
            let second_status = second.status();
            if first_status.state == BookState::Valid && second_status.state == BookState::Valid {
                return Ok(());
            }
            tokio::select! {
                status = first.changed() => { status?; }
                status = second.changed() => { status?; }
            }
        }
    })
    .await
    .context("timed out waiting for both live order books")?
}

async fn run_reconnect_smoke(
    first: &mut BookFeed,
    second: &mut BookFeed,
    recovery_timeout: Duration,
) -> Result<()> {
    let first_before = first.status();
    let second_before = second.status();
    tokio::try_join!(first.force_reconnect(), second.force_reconnect())?;
    let (first_after, second_after) = tokio::try_join!(
        wait_for_rebuild(first, first_before.generation, recovery_timeout),
        wait_for_rebuild(second, second_before.generation, recovery_timeout),
    )?;
    println!(
        "RECONNECT_OK {} generation={}->{} reconnects={} updates={} sequence={}",
        first_after.venue,
        first_before.generation,
        first_after.generation,
        first_after.reconnects,
        first_after.applied_updates,
        first_after
            .snapshot
            .as_ref()
            .expect("VALID feed has snapshot")
            .sequence,
    );
    println!(
        "RECONNECT_OK {} generation={}->{} reconnects={} updates={} sequence={}",
        second_after.venue,
        second_before.generation,
        second_after.generation,
        second_after.reconnects,
        second_after.applied_updates,
        second_after
            .snapshot
            .as_ref()
            .expect("VALID feed has snapshot")
            .sequence,
    );
    Ok(())
}

async fn wait_for_rebuild(
    feed: &mut BookFeed,
    previous_generation: u64,
    recovery_timeout: Duration,
) -> Result<BookFeedStatus> {
    let venue = feed.status().venue;
    timeout(recovery_timeout, async {
        loop {
            let status = feed.status();
            if status.generation > previous_generation && status.state == BookState::Valid {
                return Ok(status);
            }
            feed.changed().await?;
        }
    })
    .await
    .with_context(|| format!("timed out waiting for {venue} order book rebuild"))?
}

fn scan_current(
    config: &ObserverConfig,
    first_status: &BookFeedStatus,
    second_status: &BookFeedStatus,
    first_instrument: &InstrumentSpec,
    second_instrument: &InstrumentSpec,
    accounts: &[AccountData; 2],
) -> Result<DecisionEvent> {
    if first_status.state != BookState::Valid || second_status.state != BookState::Valid {
        bail!(
            "live books unavailable: {}={} ({}) {}={} ({})",
            first_status.venue,
            state_name(first_status.state),
            first_status.reason.as_deref().unwrap_or("no reason"),
            second_status.venue,
            state_name(second_status.state),
            second_status.reason.as_deref().unwrap_or("no reason"),
        );
    }
    let first_book = first_status
        .snapshot
        .as_deref()
        .context("first VALID feed omitted its snapshot")?;
    let second_book = second_status
        .snapshot
        .as_deref()
        .context("second VALID feed omitted its snapshot")?;
    let now_ms = unix_timestamp_ms()?;
    let admission_rejections = accounts
        .iter()
        .flat_map(|account| account.admission_rejections(now_ms))
        .collect::<Vec<_>>();
    let report = scan_pair(ScanInput {
        first_book,
        second_book,
        instruments: InstrumentPair {
            first: first_instrument,
            second: second_instrument,
        },
        quantity: config.quantity,
        fees: VenueFees {
            first: VenueFeeRates {
                buy_taker_rate: accounts[0].fee.buy_taker_rate,
                sell_taker_rate: accounts[0].fee.sell_taker_rate,
            },
            second: VenueFeeRates {
                buy_taker_rate: accounts[1].fee.buy_taker_rate,
                sell_taker_rate: accounts[1].fee.sell_taker_rate,
            },
        },
        strategy: &config.strategy,
        now_ms,
        freshness: FreshnessLimits {
            max_snapshot_age_ms: config.max_snapshot_age_ms,
            max_pair_skew_ms: config.max_pair_skew_ms,
        },
        admission_rejections: &admission_rejections,
    })?;
    DecisionEvent::capture(
        config,
        [first_status, second_status],
        [first_instrument, second_instrument],
        accounts,
        now_ms,
        admission_rejections,
        report,
    )
}

fn submit_archive(archive: &mut ArchiveWriter, event: ArchivedEvent) {
    if let Err(error) = archive.submit(event) {
        eprintln!("archive submit failed; observation continues: {error:#}");
    }
}

fn state_name(state: BookState) -> &'static str {
    match state {
        BookState::Syncing => "SYNCING",
        BookState::Valid => "VALID",
        BookState::Stale => "STALE",
        BookState::Invalid => "INVALID",
    }
}

#[derive(Serialize)]
struct ObserverOutput<'a> {
    accounts: &'a [AccountData; 2],
    report: &'a ScanReport,
}

fn print_accounts(accounts: &[AccountData; 2], format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string(accounts)?),
        OutputFormat::Text => {
            for account in accounts {
                print_account(account);
            }
        }
    }
    Ok(())
}

fn print_report(
    report: &ScanReport,
    accounts: &[AccountData; 2],
    format: OutputFormat,
) -> Result<()> {
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string(&ObserverOutput { accounts, report })?
        ),
        OutputFormat::Text => {
            for account in accounts {
                print_account(account);
            }
            println!(
                "{} quantity={} receive_skew={}ms",
                report.symbol, report.quantity, report.pair_received_skew_ms
            );
            for opportunity in &report.directions {
                let decision = if opportunity.accepted {
                    "ACCEPT"
                } else {
                    "REJECT"
                };
                println!(
                    "  {} buy={} sell={} buy_vwap={} sell_vwap={} gross={} fees={} expected_net={} admission={} ({} bps)",
                    decision,
                    opportunity.buy_venue,
                    opportunity.sell_venue,
                    opportunity.buy_vwap,
                    opportunity.sell_vwap,
                    opportunity.gross_profit,
                    opportunity.fees,
                    opportunity.expected_net_profit,
                    opportunity.admission_profit,
                    opportunity.admission_net_bps,
                );
                for reason in &opportunity.rejection_reasons {
                    println!("    - {reason}");
                }
            }
        }
    }
    Ok(())
}

fn print_shadow_report(report: &ShadowReport, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string(report)?),
        OutputFormat::Text => {
            println!(
                "REPLAY_OK records={} decisions={} health={} duration={}ms online={}ms invalid={}ms positive_net={} accepted={} gaps={} dropped={} bytes={} ignored_tails={}/{}",
                report.records,
                report.decision_records,
                report.health_records,
                report.observed_duration_ms,
                report.online_duration_ms,
                report.invalid_duration_ms,
                report.positive_net_opportunities,
                report.accepted_opportunities,
                report.gap_records,
                report.dropped_events,
                report.archive_bytes,
                report.ignored_incomplete_tail_bytes,
                report.ignored_gap_tail_bytes,
            );
            println!("  reconnects={:?}", report.reconnects);
            println!("  net_profit={:?}", report.net_profit_distribution);
            println!("  visible_capacity={:?}", report.visible_capacity);
            println!("  rejection_reasons={:?}", report.rejection_reason_counts);
            for sample in &report.tail_samples {
                println!(
                    "  tail event={} buy={} sell={} admission={} bps={}",
                    sample.event_id,
                    sample.buy_venue,
                    sample.sell_venue,
                    sample.admission_profit,
                    sample.admission_net_bps,
                );
            }
        }
    }
    Ok(())
}

fn print_paper_core_smoke(report: &PaperCoreSmokeReport, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string(report)?),
        OutputFormat::Text => println!(
            "PAPER_CORE_OK schema={} lock_rejected={} competition={}/{} rollback_rows={} idempotent={} conflict_rejected={} recovered={}/{}/{}/{}/{} audit_immutable={} external_order_calls={}",
            report.schema_version,
            report.same_domain_lock_rejected,
            report.concurrent_successes,
            report.concurrent_attempts,
            report.rejected_request_rows,
            report.idempotent_replay,
            report.conflicting_replay_rejected,
            report.recovered_plans,
            report.recovered_risk_decisions,
            report.recovered_intents,
            report.recovered_reservations,
            report.recovered_audit_events,
            report.audit_events_immutable,
            report.external_order_calls,
        ),
    }
    Ok(())
}

fn print_order_facts_smoke(report: &OrderFactsSmokeReport, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string(report)?),
        OutputFormat::Text => println!(
            "ORDER_FACTS_OK schema={} timeout_unknown={} not_found_unknown={} found_recovered={} duplicate_ignored={} cancel_trade_preserved={} recovered_filled={} status={:?} cancel={:?} external_order_calls={}",
            report.schema_version,
            report.submit_timeout_unknown,
            report.query_not_found_preserved_unknown,
            report.query_found_recovered,
            report.duplicate_trade_ignored,
            report.cancel_race_trade_preserved,
            report.recovered_filled_quantity,
            report.recovered_submission_status,
            report.recovered_cancel_status,
            report.external_order_calls,
        ),
    }
    Ok(())
}

fn print_account(account: &AccountData) {
    println!(
        "ACCOUNT {} fee_source={} buy_taker={} sell_taker={} loaded_at={:?} expires_at={:?} read_only={:?} region_confirmed={} account_confirmed={}",
        account.venue,
        account.fee.source,
        account.fee.buy_taker_rate,
        account.fee.sell_taker_rate,
        account.fee.loaded_at_ms,
        account.fee.expires_at_ms,
        account.permission.read_only,
        account.capability.region_eligible_confirmed,
        account.capability.account_eligible_confirmed,
    );
    for reason in account.admission_rejections(unix_timestamp_ms().unwrap_or(u64::MAX)) {
        println!("  - {reason}");
    }
}
