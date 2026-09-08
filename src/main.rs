use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use personal_taoli::{
    config::ObserverConfig,
    instrument::{InstrumentSpec, validate_pair},
    local_book::{BookFeed, BookFeedStatus, BookState},
    market::unix_timestamp_ms,
    scan::{FreshnessLimits, InstrumentPair, ScanInput, ScanReport, VenueFees, scan_pair},
    venues::{BinanceMarketData, BybitMarketData, MarketDataVenue},
};
use reqwest::Client;
use tokio::time::{MissedTickBehavior, interval, timeout};

#[derive(Debug, Parser)]
#[command(version, about = "Read-only Binance/Bybit spot arbitrage observer")]
struct Cli {
    #[arg(short, long, default_value = "config/observer.toml")]
    config: PathBuf,
    #[arg(long, help = "Wait for live books and scan exactly once")]
    once: bool,
    #[arg(
        long,
        conflicts_with = "once",
        help = "Force both live feeds to reconnect, verify recovery, then exit"
    )]
    reconnect_smoke: bool,
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
    let config = ObserverConfig::load(&cli.config)?;
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
        client,
        &config.bybit.base_url,
        &config.bybit.websocket_url,
    )?);
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
        let report = scan_current(
            &config,
            &binance_feed.status(),
            &bybit_feed.status(),
            &binance_instrument,
            &bybit_instrument,
        )?;
        print_report(&report, cli.output)?;
        return Ok(());
    }

    eprintln!(
        "OBSERVE mode: symbol={} quantity={} {} (live public feeds, no API keys, no orders)",
        config.symbol, config.quantity, config.base_asset
    );
    let mut ticker = interval(config.poll_interval());
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
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
                ) {
                    Ok(report) => print_report(&report, cli.output)?,
                    Err(error) => eprintln!("scan skipped: {error:#}"),
                }
            }
            result = tokio::signal::ctrl_c() => {
                result.context("failed to listen for Ctrl-C")?;
                eprintln!("observer stopped");
                break;
            }
        }
    }
    Ok(())
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
) -> Result<ScanReport> {
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
    scan_pair(ScanInput {
        first_book,
        second_book,
        instruments: InstrumentPair {
            first: first_instrument,
            second: second_instrument,
        },
        quantity: config.quantity,
        fees: VenueFees {
            first: config.binance.taker_fee_rate,
            second: config.bybit.taker_fee_rate,
        },
        strategy: &config.strategy,
        now_ms: unix_timestamp_ms()?,
        freshness: FreshnessLimits {
            max_snapshot_age_ms: config.max_snapshot_age_ms,
            max_pair_skew_ms: config.max_pair_skew_ms,
        },
    })
}

fn state_name(state: BookState) -> &'static str {
    match state {
        BookState::Syncing => "SYNCING",
        BookState::Valid => "VALID",
        BookState::Stale => "STALE",
        BookState::Invalid => "INVALID",
    }
}

fn print_report(report: &ScanReport, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string(report)?),
        OutputFormat::Text => {
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
