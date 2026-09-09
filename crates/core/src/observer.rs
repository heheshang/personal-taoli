use std::{future::Future, path::Path, sync::Arc, time::Duration};

use crate::{
    account::{AccountData, load_account_data},
    archive::{
        ArchiveWriter, ArchivedEvent, DecisionEvent, FeedVersion, HealthEvent, default_gap_path,
    },
    config::{ObserverConfig, PairConfig},
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

/// 单交易对重连烟测结果（多币种：每对一条）。
#[derive(Debug)]
pub struct ReconnectSmokeOutcome {
    pub symbol: String,
    pub binance: BookFeedStatus,
    pub bybit: BookFeedStatus,
}

/// 共享 HTTP client 与两所 venue 实例（配置级，非逐对）。
type SharedVenues = (Arc<dyn MarketDataVenue>, Arc<dyn MarketDataVenue>, Client);

fn build_venues(config: &ObserverConfig) -> Result<SharedVenues> {
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
    Ok((binance, bybit, client))
}

/// 为单个交易对加载账户费率与两所合约规格并校验；返回 (accounts, first_instrument, second_instrument)。
async fn load_pair_context(
    client: &Client,
    binance: &dyn MarketDataVenue,
    bybit: &dyn MarketDataVenue,
    pair: &PairConfig,
    config: &ObserverConfig,
) -> Result<([AccountData; 2], InstrumentSpec, InstrumentSpec)> {
    let accounts = load_account_data(
        client,
        &pair.symbol,
        &config.binance,
        &config.bybit,
        config.auth_recv_window_ms,
        config.max_fee_age_ms,
    )
    .await;
    let (first_instrument, second_instrument) = tokio::try_join!(
        binance.load_instrument(&pair.symbol),
        bybit.load_instrument(&pair.symbol),
    )?;
    validate_pair(
        &first_instrument,
        &second_instrument,
        &pair.symbol,
        &pair.base_asset,
        &pair.quote_asset,
        pair.quantity,
    )?;
    Ok((accounts, first_instrument, second_instrument))
}

/// 订阅单个交易对的两所订单簿并等待双方 VALID。
async fn subscribe_pair_books(
    binance: &dyn MarketDataVenue,
    bybit: &dyn MarketDataVenue,
    pair: &PairConfig,
    config: &ObserverConfig,
) -> Result<(BookFeed, BookFeed)> {
    let stale_after = Duration::from_millis(config.max_snapshot_age_ms);
    let mut first = binance.subscribe_order_book(
        &pair.symbol,
        config.orderbook_depth,
        stale_after,
        config.reconnect_delay(),
    )?;
    let mut second = bybit.subscribe_order_book(
        &pair.symbol,
        config.orderbook_depth,
        stale_after,
        config.reconnect_delay(),
    )?;
    wait_for_valid_pair(&mut first, &mut second, config.stream_start_timeout()).await?;
    Ok((first, second))
}

/// Brings both public order-book feeds VALID, forces one reconnect on each,
/// then waits until both recover on a newer generation, for every configured pair.
///
/// Read-only: no account credentials are used and no order side effects exist.
pub async fn run_reconnect_smoke(config: &ObserverConfig) -> Result<Vec<ReconnectSmokeOutcome>> {
    let (binance, bybit, _client) = build_venues(config)?;
    let pairs = config.effective_pairs().to_vec();
    let mut outcomes = Vec::with_capacity(pairs.len());
    for pair in &pairs {
        let (mut first, mut second) =
            subscribe_pair_books(binance.as_ref(), bybit.as_ref(), pair, config).await?;
        let before = [first.status(), second.status()];
        first.force_reconnect().await?;
        second.force_reconnect().await?;
        let recovered = timeout(config.stream_start_timeout(), async {
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
        .with_context(|| {
            format!(
                "timed out waiting for {} feeds to recover after reconnect",
                pair.symbol
            )
        })??;
        let [binance_status, bybit_status] = recovered;
        outcomes.push(ReconnectSmokeOutcome {
            symbol: pair.symbol.clone(),
            binance: binance_status,
            bybit: bybit_status,
        });
    }
    Ok(outcomes)
}

/// 对每个配置的交易对执行一次完整观察；全部成功才写档，任一失败整体返回错误。
pub async fn observe_once(
    config: &ObserverConfig,
    archive_path: &Path,
) -> Result<Vec<ObservationRun>> {
    let gap_path = default_gap_path(archive_path);
    let (binance, bybit, client) = build_venues(config)?;
    let mut runs = Vec::with_capacity(config.effective_pairs().len());
    let mut events = Vec::with_capacity(config.effective_pairs().len());
    for pair in config.effective_pairs() {
        let (accounts, first_instrument, second_instrument) =
            load_pair_context(&client, binance.as_ref(), bybit.as_ref(), pair, config).await?;
        let (first, second) =
            subscribe_pair_books(binance.as_ref(), bybit.as_ref(), pair, config).await?;
        let first_status = first.status();
        let second_status = second.status();
        let event = scan_current(
            pair,
            config,
            &first_status,
            &second_status,
            &first_instrument,
            &second_instrument,
            &accounts,
        )?;
        runs.push(ObservationRun {
            report: event.report.clone(),
            accounts,
            feeds: [first_status, second_status],
            archived: false,
        });
        events.push(event);
    }
    let mut archive = ArchiveWriter::start(
        archive_path,
        &gap_path,
        config.archive.queue_capacity,
        config.archive.raw_retention_days,
    )?;
    for event in events {
        archive.submit(ArchivedEvent::Decision(Box::new(event)))?;
    }
    archive.finish()?;
    for run in &mut runs {
        run.archived = true;
    }
    Ok(runs)
}

/// 单个交易对的全生命周期观察状态：双边订单簿、合约规格与账户费率。
struct PairRun {
    pair: PairConfig,
    first: BookFeed,
    second: BookFeed,
    first_instrument: InstrumentSpec,
    second_instrument: InstrumentSpec,
    accounts: [AccountData; 2],
}

impl PairRun {
    async fn new(
        binance: &dyn MarketDataVenue,
        bybit: &dyn MarketDataVenue,
        client: &Client,
        pair: &PairConfig,
        config: &ObserverConfig,
    ) -> Result<Self> {
        let (accounts, first_instrument, second_instrument) =
            load_pair_context(client, binance, bybit, pair, config).await?;
        let (first, second) = subscribe_pair_books(binance, bybit, pair, config).await?;
        Ok(Self {
            pair: pair.clone(),
            first,
            second,
            first_instrument,
            second_instrument,
            accounts,
        })
    }
}

/// Runs the read-only observer until `shutdown` resolves.
///
/// A single loop scans every configured pair each tick; each pair keeps its own
/// books, instruments and account fees. Invalid feed states become per-pair health
/// records; they never produce a decision record.
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
    let (binance, bybit, client) = build_venues(config)?;
    let pairs = config.effective_pairs().to_vec();
    if pairs.is_empty() {
        bail!("pairs must not be empty");
    }
    let mut shutdown = Box::pin(shutdown);
    let mut runs = {
        let startup = Box::pin(async {
            let mut runs = Vec::with_capacity(pairs.len());
            for pair in &pairs {
                runs.push(
                    PairRun::new(binance.as_ref(), bybit.as_ref(), &client, pair, config).await?,
                );
            }
            anyhow::Ok(runs)
        });
        tokio::select! {
            result = startup => result?,
            _ = &mut shutdown => return Ok(()),
        }
    };
    let mut archive = ArchiveWriter::start(
        archive_path,
        &gap_path,
        config.archive.queue_capacity,
        config.archive.raw_retention_days,
    )?;
    let mut scan_ticker = tokio::time::interval(config.poll_interval());
    let mut account_ticker = tokio::time::interval(config.account_refresh_interval());
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            _ = account_ticker.tick() => {
                for run in &mut runs {
                    run.accounts = load_account_data(
                        &client,
                        &run.pair.symbol,
                        &config.binance,
                        &config.bybit,
                        config.auth_recv_window_ms,
                        config.max_fee_age_ms,
                    ).await;
                }
            }
            _ = scan_ticker.tick() => {
                for run in &mut runs {
                    let first_status = run.first.status();
                    let second_status = run.second.status();
                    if first_status.state == BookState::Valid
                        && second_status.state == BookState::Valid
                    {
                        let event = scan_current(
                            &run.pair,
                            config,
                            &first_status,
                            &second_status,
                            &run.first_instrument,
                            &run.second_instrument,
                            &run.accounts,
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
    pair: &PairConfig,
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
        quantity: pair.quantity,
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
        pair,
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
