//! 归档事件类型定义。

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{
    account::AccountData,
    config::{ObserverConfig, StrategyConfig},
    instrument::InstrumentSpec,
    local_book::{BookFeedStatus, BookState},
    market::{OrderBookSnapshot, unix_timestamp_ms},
    scan::{FreshnessLimits, ScanInput, ScanReport, VenueFeeRates, VenueFees, scan_pair},
};

use super::SCHEMA_VERSION;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedVersion {
    pub venue: String,
    pub symbol: String,
    pub state: BookState,
    pub generation: u64,
    pub reconnects: u64,
    pub applied_updates: u64,
    pub reason: Option<String>,
}

impl From<&BookFeedStatus> for FeedVersion {
    fn from(status: &BookFeedStatus) -> Self {
        Self {
            venue: status.venue.clone(),
            symbol: status.symbol.clone(),
            state: status.state,
            generation: status.generation,
            reconnects: status.reconnects,
            applied_updates: status.applied_updates,
            reason: status.reason.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchivedFeeVersion {
    pub venue: String,
    pub symbol: String,
    pub buy_taker_rate: Decimal,
    pub sell_taker_rate: Decimal,
    pub source: String,
    pub loaded_at_ms: Option<u64>,
    pub expires_at_ms: Option<u64>,
    pub actual_account_rate: bool,
}

impl From<&AccountData> for ArchivedFeeVersion {
    fn from(account: &AccountData) -> Self {
        Self {
            venue: account.venue.to_owned(),
            symbol: account.fee.symbol.clone(),
            buy_taker_rate: account.fee.buy_taker_rate,
            sell_taker_rate: account.fee.sell_taker_rate,
            source: account.fee.source.to_owned(),
            loaded_at_ms: account.fee.loaded_at_ms,
            expires_at_ms: account.fee.expires_at_ms,
            actual_account_rate: account.fee.actual_account_rate,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionConfig {
    pub symbol: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub quantity: Decimal,
    pub strategy: StrategyConfig,
    pub freshness: FreshnessLimits,
}

impl From<&ObserverConfig> for DecisionConfig {
    fn from(config: &ObserverConfig) -> Self {
        Self {
            symbol: config.symbol.clone(),
            base_asset: config.base_asset.clone(),
            quote_asset: config.quote_asset.clone(),
            quantity: config.quantity,
            strategy: config.strategy.clone(),
            freshness: FreshnessLimits {
                max_snapshot_age_ms: config.max_snapshot_age_ms,
                max_pair_skew_ms: config.max_pair_skew_ms,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionEvent {
    pub evaluated_at_ms: u64,
    pub feeds: [FeedVersion; 2],
    pub books: [OrderBookSnapshot; 2],
    pub instruments: [InstrumentSpec; 2],
    pub fees: [ArchivedFeeVersion; 2],
    pub config_version: String,
    pub config: DecisionConfig,
    pub admission_rejections: Vec<String>,
    pub report: ScanReport,
}

impl DecisionEvent {
    pub fn capture(
        config: &ObserverConfig,
        statuses: [&BookFeedStatus; 2],
        instruments: [&InstrumentSpec; 2],
        accounts: &[AccountData; 2],
        evaluated_at_ms: u64,
        admission_rejections: Vec<String>,
        report: ScanReport,
    ) -> anyhow::Result<Self> {
        use anyhow::Context;

        let first_book = statuses[0]
            .snapshot
            .as_deref()
            .context("first VALID feed omitted its snapshot")?;
        let second_book = statuses[1]
            .snapshot
            .as_deref()
            .context("second VALID feed omitted its snapshot")?;
        let decision_config = DecisionConfig::from(config);
        let config_version = content_version(&decision_config)?;
        Ok(Self {
            evaluated_at_ms,
            feeds: [statuses[0].into(), statuses[1].into()],
            books: [first_book.clone(), second_book.clone()],
            instruments: [instruments[0].clone(), instruments[1].clone()],
            fees: [(&accounts[0]).into(), (&accounts[1]).into()],
            config_version,
            config: decision_config,
            admission_rejections,
            report,
        })
    }

    pub(crate) fn replay(&self) -> anyhow::Result<ScanReport> {
        use anyhow::bail;

        if content_version(&self.config)? != self.config_version {
            bail!("decision config content does not match config_version");
        }
        if self.feeds.iter().any(|feed| feed.state != BookState::Valid) {
            bail!("decision record contains a non-VALID feed");
        }
        for (feed, book) in self.feeds.iter().zip(&self.books) {
            if feed.venue != book.venue || feed.symbol != book.symbol {
                bail!("decision feed identity does not match its order book");
            }
        }
        scan_pair(ScanInput {
            first_book: &self.books[0],
            second_book: &self.books[1],
            instruments: crate::scan::InstrumentPair {
                first: &self.instruments[0],
                second: &self.instruments[1],
            },
            quantity: self.config.quantity,
            fees: VenueFees {
                first: VenueFeeRates {
                    buy_taker_rate: self.fees[0].buy_taker_rate,
                    sell_taker_rate: self.fees[0].sell_taker_rate,
                },
                second: VenueFeeRates {
                    buy_taker_rate: self.fees[1].buy_taker_rate,
                    sell_taker_rate: self.fees[1].sell_taker_rate,
                },
            },
            strategy: &self.config.strategy,
            now_ms: self.evaluated_at_ms,
            freshness: self.config.freshness,
            admission_rejections: &self.admission_rejections,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthEvent {
    pub observed_at_ms: u64,
    pub feeds: [FeedVersion; 2],
    pub skip_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event_type", content = "event", rename_all = "snake_case")]
pub enum ArchivedEvent {
    Decision(Box<DecisionEvent>),
    Health(Box<HealthEvent>),
}

impl ArchivedEvent {
    pub(crate) fn observed_at_ms(&self) -> u64 {
        match self {
            Self::Decision(event) => event.evaluated_at_ms,
            Self::Health(event) => event.observed_at_ms,
        }
    }

    pub(crate) fn feeds(&self) -> &[FeedVersion; 2] {
        match self {
            Self::Decision(event) => &event.feeds,
            Self::Health(event) => &event.feeds,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveRecord {
    pub schema_version: u16,
    pub event_id: String,
    pub run_id: String,
    pub ordinal: u64,
    pub code_version: String,
    pub recorded_at_ms: u64,
    pub raw_retention_days: u16,
    #[serde(flatten)]
    pub event: ArchivedEvent,
}

impl ArchiveRecord {
    pub(crate) fn new(
        run_id: &str,
        ordinal: u64,
        raw_retention_days: u16,
        event: ArchivedEvent,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            event_id: format!("{run_id}:{ordinal}"),
            ordinal,
            run_id: run_id.to_owned(),
            code_version: env!("CARGO_PKG_VERSION").to_owned(),
            recorded_at_ms: unix_timestamp_ms()?,
            raw_retention_days,
            event,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveGap {
    pub schema_version: u16,
    pub run_id: String,
    pub first_event_id: String,
    pub last_event_id: String,
    pub first_observed_at_ms: u64,
    pub last_observed_at_ms: u64,
    pub dropped_events: u64,
    pub reason: String,
    pub recorded_at_ms: u64,
}

#[derive(Debug)]
pub(crate) struct PendingGap {
    pub first_event_id: String,
    pub last_event_id: String,
    pub first_observed_at_ms: u64,
    pub last_observed_at_ms: u64,
    pub dropped_events: u64,
}

impl PendingGap {
    pub(crate) fn from_record(record: &ArchiveRecord) -> Self {
        let observed_at_ms = record.event.observed_at_ms();
        Self {
            first_event_id: record.event_id.clone(),
            last_event_id: record.event_id.clone(),
            first_observed_at_ms: observed_at_ms,
            last_observed_at_ms: observed_at_ms,
            dropped_events: 1,
        }
    }

    pub(crate) fn extend(&mut self, record: &ArchiveRecord) {
        self.last_event_id.clone_from(&record.event_id);
        self.last_observed_at_ms = record.event.observed_at_ms();
        self.dropped_events += 1;
    }

    pub(crate) fn finish(self, run_id: &str, reason: &str) -> ArchiveGap {
        ArchiveGap {
            schema_version: SCHEMA_VERSION,
            run_id: run_id.to_owned(),
            first_event_id: self.first_event_id,
            last_event_id: self.last_event_id,
            first_observed_at_ms: self.first_observed_at_ms,
            last_observed_at_ms: self.last_observed_at_ms,
            dropped_events: self.dropped_events,
            reason: reason.to_owned(),
            recorded_at_ms: unix_timestamp_ms().unwrap_or(u64::MAX),
        }
    }
}

pub(crate) fn content_version<T: Serialize>(value: &T) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};

    let encoded = serde_json::to_vec(value)?;
    let digest = Sha256::digest(encoded);
    Ok(format!("sha256:{digest:x}"))
}
