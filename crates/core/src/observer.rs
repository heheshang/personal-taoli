use std::{future::Future, path::Path, sync::Arc, time::Duration};

use crate::{
    account::{AccountData, load_account_data},
    archive::{
        ArchiveWriter, ArchivedEvent, DecisionEvent, FeedVersion, HealthEvent, default_gap_path,
    },
    config::ObserverConfig,
    instrument::{InstrumentSpec, validate_pair},
    local_book::{BookFeed, BookFeedStatus, BookState},
    market::unix_timestamp_ms,
    scan::{
        FreshnessLimits, InstrumentPair, ScanInput, ScanReport, VenueFeeRates, VenueFees, scan_pair,
    },
    venues::{BinanceMarketData, BybitMarketData, MarketDataVenue},
};
use anyhow::{Context, Result, bail};
use reqwest::Client;
use tokio::time::timeout;

#[derive(Debug)]
pub struct ObservationRun {
    pub report: ScanReport,
    pub accounts: [AccountData; 2],
    pub feeds: [BookFeedStatus; 2],
    pub archived: bool,
}

/// Outcome of a read-only reconnect smoke over both live public feeds.
#[derive(Debug)]
pub struct ReconnectSmokeOutcome {
    pub binance: BookFeedStatus,
    pub bybit: BookFeedStatus,
}

/// Brings both public order-book feeds VALID, forces one reconnect on each,
/// then waits until both recover on a newer generation.
///
/// Read-only: no account credentials are used and no order side effects exist.
pub async fn run_reconnect_smoke(config: &ObserverConfig) -> Result<ReconnectSmokeOutcome> {
    let client = build_http_client(config)?;
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
    let stale_after = Duration::from_millis(config.max_snapshot_age_ms);
    let mut first = binance.subscribe_order_book(
        &config.symbol,
        config.orderbook_depth,
        stale_after,
        config.reconnect_delay(),
    )?;
    let mut second = bybit.subscribe_order_book(
        &config.symbol,
        config.orderbook_depth,
        stale_after,
        config.reconnect_delay(),
    )?;
    wait_for_valid_pair(&mut first, &mut second, config.stream_start_timeout()).await?;
    let before = [first.status(), second.status()];
    first.force_reconnect().await?;
    second.force_reconnect().await?;
    let recovered =
        timeout(config.stream_start_timeout(), async {
            loop {
                let current = [first.status(), second.status()];
                if current.iter().zip(&before).all(|(now, old)| {
                    now.state == BookState::Valid && now.generation > old.generation
                }) {
                    return Ok::<_, anyhow::Error>(current);
                }
                tokio::select! {
                    status = first.changed() => { status?; }
                    status = second.changed() => { status?; }
                }
            }
        })
        .await
        .context("timed out waiting for both feeds to recover after reconnect")??;
    let [binance, bybit] = recovered;
    Ok(ReconnectSmokeOutcome { binance, bybit })
}

pub async fn observe_once(config: &ObserverConfig, archive_path: &Path) -> Result<ObservationRun> {
    let gap_path = default_gap_path(archive_path);
    let client = build_http_client(config)?;
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
    let accounts = load_account_data(
        &client,
        &config.symbol,
        &config.binance,
        &config.bybit,
        config.auth_recv_window_ms,
        config.max_fee_age_ms,
    )
    .await;
    let (first_instrument, second_instrument) = tokio::try_join!(
        binance.load_instrument(&config.symbol),
        bybit.load_instrument(&config.symbol),
    )?;
    validate_pair(
        &first_instrument,
        &second_instrument,
        &config.symbol,
        &config.base_asset,
        &config.quote_asset,
        config.quantity,
    )?;

    let stale_after = Duration::from_millis(config.max_snapshot_age_ms);
    let mut first = binance.subscribe_order_book(
        &config.symbol,
        config.orderbook_depth,
        stale_after,
        config.reconnect_delay(),
    )?;
    let mut second = bybit.subscribe_order_book(
        &config.symbol,
        config.orderbook_depth,
        stale_after,
        config.reconnect_delay(),
    )?;
    wait_for_valid_pair(&mut first, &mut second, config.stream_start_timeout()).await?;

    let first_status = first.status();
    let second_status = second.status();
    let event = scan_current(
        config,
        &first_status,
        &second_status,
        &first_instrument,
        &second_instrument,
        &accounts,
    )?;
    let report = event.report.clone();
    let mut archive = ArchiveWriter::start(
        archive_path,
        &gap_path,
        config.archive.queue_capacity,
        config.archive.raw_retention_days,
    )?;
    archive.submit(ArchivedEvent::Decision(Box::new(event)))?;
    archive.finish()?;

    Ok(ObservationRun {
        report,
        accounts,
        feeds: [first_status, second_status],
        archived: true,
    })
}

/// Runs the read-only observer until `shutdown` resolves.
///
/// The market feeds and archive writer live for the whole run. Invalid feed states
/// become health records; they never produce a decision record.
pub async fn observe_continuously<F, S>(
    config: &ObserverConfig,
    archive_path: &Path,
    mut on_report: F,
    shutdown: S,
) -> Result<()>
where
    F: FnMut(&ScanReport),
    S: Future<Output = ()>,
{
    let gap_path = default_gap_path(archive_path);
    let client = build_http_client(config)?;
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
    let (first_instrument, second_instrument) = tokio::try_join!(
        binance.load_instrument(&config.symbol),
        bybit.load_instrument(&config.symbol),
    )?;
    validate_pair(
        &first_instrument,
        &second_instrument,
        &config.symbol,
        &config.base_asset,
        &config.quote_asset,
        config.quantity,
    )?;

    let stale_after = Duration::from_millis(config.max_snapshot_age_ms);
    let mut first = binance.subscribe_order_book(
        &config.symbol,
        config.orderbook_depth,
        stale_after,
        config.reconnect_delay(),
    )?;
    let mut second = bybit.subscribe_order_book(
        &config.symbol,
        config.orderbook_depth,
        stale_after,
        config.reconnect_delay(),
    )?;
    let mut archive = ArchiveWriter::start(
        archive_path,
        &gap_path,
        config.archive.queue_capacity,
        config.archive.raw_retention_days,
    )?;
    let mut shutdown = Box::pin(shutdown);
    tokio::select! {
        result = wait_for_valid_pair(&mut first, &mut second, config.stream_start_timeout()) => result?,
        _ = &mut shutdown => {
            archive.finish()?;
            return Ok(());
        }
    }

    let mut scan_ticker = tokio::time::interval(config.poll_interval());
    let mut account_ticker = tokio::time::interval(config.account_refresh_interval());
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
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
            _ = scan_ticker.tick() => {
                let first_status = first.status();
                let second_status = second.status();
                if first_status.state == BookState::Valid && second_status.state == BookState::Valid {
                    let event = scan_current(
                        config,
                        &first_status,
                        &second_status,
                        &first_instrument,
                        &second_instrument,
                        &accounts,
                    )?;
                    on_report(&event.report);
                    archive.submit(ArchivedEvent::Decision(Box::new(event)))?;
                } else {
                    let observed_at_ms = unix_timestamp_ms()?;
                    archive.submit(ArchivedEvent::Health(Box::new(HealthEvent {
                        observed_at_ms,
                        feeds: [&first_status, &second_status].map(FeedVersion::from),
                        skip_reason: format!(
                            "live books unavailable: {}={} {}={}",
                            first_status.venue,
                            state_name(first_status.state),
                            second_status.venue,
                            state_name(second_status.state),
                        ),
                    })))?;
                }
            }
        }
    }
    archive.finish()
}

fn build_http_client(config: &ObserverConfig) -> Result<Client> {
    Client::builder()
        .timeout(config.http_timeout())
        .user_agent(concat!("personal-taoli/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to build HTTP client")
}

async fn wait_for_valid_pair(
    first: &mut BookFeed,
    second: &mut BookFeed,
    start_timeout: Duration,
) -> Result<()> {
    timeout(start_timeout, async {
        loop {
            if first.status().state == BookState::Valid && second.status().state == BookState::Valid
            {
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
            second_status.reason.as_deref().unwrap_or("no reason")
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

fn state_name(state: BookState) -> &'static str {
    match state {
        BookState::Syncing => "SYNCING",
        BookState::Valid => "VALID",
        BookState::Stale => "STALE",
        BookState::Invalid => "INVALID",
    }
}
