use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

use anyhow::{Context, Result, bail};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    account::AccountData,
    config::{ObserverConfig, StrategyConfig},
    instrument::InstrumentSpec,
    local_book::{BookFeedStatus, BookState},
    market::{OrderBookSnapshot, unix_timestamp_ms},
    scan::{
        FreshnessLimits, InstrumentPair, ScanInput, ScanReport, VenueFeeRates, VenueFees, scan_pair,
    },
};

const SCHEMA_VERSION: u16 = 1;
const TAIL_SAMPLE_LIMIT: usize = 5;
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(1);

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
    ) -> Result<Self> {
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

    fn replay(&self) -> Result<ScanReport> {
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
            instruments: InstrumentPair {
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
    fn observed_at_ms(&self) -> u64 {
        match self {
            Self::Decision(event) => event.evaluated_at_ms,
            Self::Health(event) => event.observed_at_ms,
        }
    }

    fn feeds(&self) -> &[FeedVersion; 2] {
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
    fn new(
        run_id: &str,
        ordinal: u64,
        raw_retention_days: u16,
        event: ArchivedEvent,
    ) -> Result<Self> {
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
struct PendingGap {
    first_event_id: String,
    last_event_id: String,
    first_observed_at_ms: u64,
    last_observed_at_ms: u64,
    dropped_events: u64,
}

impl PendingGap {
    fn from_record(record: &ArchiveRecord) -> Self {
        let observed_at_ms = record.event.observed_at_ms();
        Self {
            first_event_id: record.event_id.clone(),
            last_event_id: record.event_id.clone(),
            first_observed_at_ms: observed_at_ms,
            last_observed_at_ms: observed_at_ms,
            dropped_events: 1,
        }
    }

    fn extend(&mut self, record: &ArchiveRecord) {
        self.last_event_id.clone_from(&record.event_id);
        self.last_observed_at_ms = record.event.observed_at_ms();
        self.dropped_events += 1;
    }

    fn finish(self, run_id: &str, reason: &str) -> ArchiveGap {
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

pub struct ArchiveWriter {
    run_id: String,
    raw_retention_days: u16,
    next_ordinal: u64,
    sender: Option<mpsc::SyncSender<ArchiveRecord>>,
    pending_overflow: Arc<Mutex<Option<PendingGap>>>,
    worker: Option<thread::JoinHandle<Result<()>>>,
}

impl ArchiveWriter {
    pub fn start(
        path: impl AsRef<Path>,
        gap_path: impl AsRef<Path>,
        queue_capacity: usize,
        raw_retention_days: u16,
    ) -> Result<Self> {
        if queue_capacity == 0 {
            bail!("archive queue capacity must be positive");
        }
        let started_at_ms = unix_timestamp_ms()?;
        let run_id = format!(
            "{started_at_ms}-{}-{}",
            std::process::id(),
            RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        );
        let (sender, receiver) = mpsc::sync_channel(queue_capacity);
        let pending_overflow = Arc::new(Mutex::new(None));
        let worker_pending = Arc::clone(&pending_overflow);
        let worker_run_id = run_id.clone();
        let archive_path = path.as_ref().to_owned();
        let gaps_path = gap_path.as_ref().to_owned();
        let mut startup_gaps = Vec::new();
        for (candidate, label) in [
            (&archive_path, "archive"),
            (&gaps_path, "archive gap journal"),
        ] {
            let discarded = repair_incomplete_tail(candidate)?;
            if discarded > 0 {
                let event_id = format!("{run_id}:startup-{label}-tail-repair");
                startup_gaps.push(ArchiveGap {
                    schema_version: SCHEMA_VERSION,
                    run_id: run_id.clone(),
                    first_event_id: event_id.clone(),
                    last_event_id: event_id,
                    first_observed_at_ms: started_at_ms,
                    last_observed_at_ms: started_at_ms,
                    dropped_events: 0,
                    reason: format!("discarded {discarded} incomplete bytes from {label}"),
                    recorded_at_ms: started_at_ms,
                });
            }
        }
        let worker = thread::Builder::new()
            .name("market-archive".into())
            .spawn(move || {
                writer_loop(
                    receiver,
                    &archive_path,
                    &gaps_path,
                    &worker_run_id,
                    worker_pending,
                    startup_gaps,
                )
            })
            .context("failed to spawn market archive writer")?;
        Ok(Self {
            run_id,
            raw_retention_days,
            next_ordinal: 1,
            sender: Some(sender),
            pending_overflow,
            worker: Some(worker),
        })
    }

    pub fn submit(&mut self, event: ArchivedEvent) -> Result<()> {
        let record = ArchiveRecord::new(
            &self.run_id,
            self.next_ordinal,
            self.raw_retention_days,
            event,
        )?;
        self.next_ordinal += 1;
        let sender = self
            .sender
            .as_ref()
            .context("archive writer is already closed")?;
        match sender.try_send(record) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(record)) => {
                let mut pending = self
                    .pending_overflow
                    .lock()
                    .map_err(|_| anyhow::anyhow!("archive gap state is poisoned"))?;
                match pending.as_mut() {
                    Some(gap) => gap.extend(&record),
                    None => *pending = Some(PendingGap::from_record(&record)),
                }
                Ok(())
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                bail!("archive writer stopped unexpectedly")
            }
        }
    }

    pub fn finish(mut self) -> Result<()> {
        self.close()
    }

    fn close(&mut self) -> Result<()> {
        self.sender.take();
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker
            .join()
            .map_err(|_| anyhow::anyhow!("archive writer thread panicked"))?
    }
}

impl Drop for ArchiveWriter {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("archive shutdown failed: {error:#}");
        }
    }
}

fn writer_loop(
    receiver: mpsc::Receiver<ArchiveRecord>,
    archive_path: &Path,
    gap_path: &Path,
    run_id: &str,
    pending_overflow: Arc<Mutex<Option<PendingGap>>>,
    startup_gaps: Vec<ArchiveGap>,
) -> Result<()> {
    let mut archive = open_append(archive_path).ok();
    let mut gaps = open_append(gap_path).ok();
    for gap in startup_gaps {
        write_gap(&mut gaps, gap_path, gap);
    }
    while let Ok(record) = receiver.recv() {
        flush_overflow_gap(&mut gaps, gap_path, run_id, &pending_overflow);
        let write_result = match archive.as_mut() {
            Some(writer) => write_json_line(writer, &record),
            None => Err(anyhow::anyhow!("archive file is unavailable")),
        };
        if let Err(error) = write_result {
            write_gap(
                &mut gaps,
                gap_path,
                PendingGap::from_record(&record)
                    .finish(run_id, &format!("archive write failed: {error:#}")),
            );
            archive = open_append(archive_path).ok();
        }
    }
    flush_overflow_gap(&mut gaps, gap_path, run_id, &pending_overflow);
    if let Some(writer) = archive.as_mut() {
        writer.flush().context("failed to flush archive")?;
    }
    if let Some(writer) = gaps.as_mut() {
        writer
            .flush()
            .context("failed to flush archive gap journal")?;
    }
    Ok(())
}

fn flush_overflow_gap(
    gaps: &mut Option<BufWriter<File>>,
    gap_path: &Path,
    run_id: &str,
    pending: &Mutex<Option<PendingGap>>,
) {
    let gap = pending.lock().ok().and_then(|mut gap| gap.take());
    if let Some(gap) = gap {
        write_gap(gaps, gap_path, gap.finish(run_id, "archive queue overflow"));
    }
}

fn write_gap(gaps: &mut Option<BufWriter<File>>, gap_path: &Path, gap: ArchiveGap) {
    if gaps.is_none() {
        *gaps = open_append(gap_path).ok();
    }
    let result = match gaps.as_mut() {
        Some(writer) => write_json_line(writer, &gap),
        None => Err(anyhow::anyhow!("gap journal is unavailable")),
    };
    if let Err(error) = result {
        *gaps = None;
        eprintln!(
            "ARCHIVE_GAP {}..{} count={} reason={} journal_error={error:#}",
            gap.first_event_id, gap.last_event_id, gap.dropped_events, gap.reason,
        );
    }
}

fn repair_incomplete_tail(path: &Path) -> Result<u64> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect archive tail {}", path.display()));
        }
    };
    if !metadata.is_file() || metadata.len() == 0 {
        return Ok(0);
    }

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("failed to open archive tail {}", path.display()))?;
    let original_len = metadata.len();
    file.seek(SeekFrom::End(-1))?;
    let mut final_byte = [0_u8; 1];
    file.read_exact(&mut final_byte)?;
    if final_byte[0] == b'\n' {
        return Ok(0);
    }

    const CHUNK_SIZE: usize = 8 * 1024;
    let mut buffer = [0_u8; CHUNK_SIZE];
    let mut cursor = original_len;
    let complete_len = loop {
        let start = cursor.saturating_sub(CHUNK_SIZE as u64);
        let len = usize::try_from(cursor - start).expect("tail chunk length fits usize");
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer[..len])?;
        if let Some(index) = buffer[..len].iter().rposition(|byte| *byte == b'\n') {
            break start + index as u64 + 1;
        }
        if start == 0 {
            break 0;
        }
        cursor = start;
    };
    file.set_len(complete_len)?;
    file.sync_data()
        .with_context(|| format!("failed to sync repaired archive tail {}", path.display()))?;
    Ok(original_len - complete_len)
}

fn open_append(path: &Path) -> Result<BufWriter<File>> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create archive directory {}", parent.display()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open archive {}", path.display()))?;
    Ok(BufWriter::new(file))
}

fn write_json_line<T: Serialize>(writer: &mut BufWriter<File>, value: &T) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn content_version<T: Serialize>(value: &T) -> Result<String> {
    let encoded = serde_json::to_vec(value)?;
    let digest = Sha256::digest(encoded);
    Ok(format!("sha256:{digest:x}"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfitDistribution {
    pub minimum: Decimal,
    pub p50: Decimal,
    pub p95: Decimal,
    pub maximum: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapacityDistribution {
    pub minimum_base_quantity: Decimal,
    pub p50_base_quantity: Decimal,
    pub p95_base_quantity: Decimal,
    pub maximum_base_quantity: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TailSample {
    pub event_id: String,
    pub buy_venue: String,
    pub sell_venue: String,
    pub admission_profit: Decimal,
    pub admission_net_bps: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShadowReport {
    pub schema_version: u16,
    pub archive_path: PathBuf,
    pub archive_bytes: u64,
    pub ignored_incomplete_tail_bytes: u64,
    pub records: u64,
    pub decision_records: u64,
    pub health_records: u64,
    pub first_event_at_ms: Option<u64>,
    pub last_event_at_ms: Option<u64>,
    pub observed_duration_ms: u64,
    pub online_duration_ms: u64,
    pub invalid_duration_ms: u64,
    pub reconnects: BTreeMap<String, u64>,
    pub direction_evaluations: u64,
    pub positive_net_opportunities: u64,
    pub accepted_opportunities: u64,
    pub net_profit_distribution: Option<ProfitDistribution>,
    pub visible_capacity: Option<CapacityDistribution>,
    pub rejection_reason_counts: BTreeMap<String, u64>,
    pub tail_samples: Vec<TailSample>,
    pub gap_records: u64,
    pub dropped_events: u64,
    pub ignored_gap_tail_bytes: u64,
    pub replayed_without_mismatch: bool,
}

pub fn replay_archive(path: impl AsRef<Path>, gap_path: impl AsRef<Path>) -> Result<ShadowReport> {
    let path = path.as_ref();
    let mut profits = Vec::new();
    let mut tails = Vec::new();
    let mut rejection_reason_counts = BTreeMap::new();
    let mut reconnects = BTreeMap::new();
    let mut run_reconnects = BTreeMap::new();
    let mut run_ordinals = BTreeMap::new();
    let mut capacities = Vec::new();
    let mut records = 0_u64;
    let mut decision_records = 0_u64;
    let mut health_records = 0_u64;
    let mut accepted_opportunities = 0_u64;
    let mut positive_net_opportunities = 0_u64;
    let mut invalid_duration_ms = 0_u64;
    let mut first_event_at_ms = None;
    let mut last_event_at_ms = None;
    let mut previous_invalid = false;

    let snapshot =
        for_each_snapshot_record(path, "archive record", false, |record: ArchiveRecord| {
            if record.schema_version != SCHEMA_VERSION {
                bail!(
                    "unsupported archive schema {} in {}",
                    record.schema_version,
                    record.event_id
                );
            }
            let observed_at_ms = record.event.observed_at_ms();
            if let Some(previous_at_ms) = last_event_at_ms {
                if observed_at_ms < previous_at_ms {
                    bail!(
                        "archive event time moved backward at {}: {} < {}",
                        record.event_id,
                        observed_at_ms,
                        previous_at_ms
                    );
                }
                if previous_invalid {
                    invalid_duration_ms = invalid_duration_ms
                        .saturating_add(observed_at_ms.saturating_sub(previous_at_ms));
                }
            } else {
                first_event_at_ms = Some(observed_at_ms);
            }
            let previous_ordinal = run_ordinals.entry(record.run_id.clone()).or_insert(0_u64);
            if record.ordinal <= *previous_ordinal {
                bail!(
                    "archive ordinal is not increasing for run {} at {}",
                    record.run_id,
                    record.event_id
                );
            }
            *previous_ordinal = record.ordinal;
            previous_invalid = matches!(&record.event, ArchivedEvent::Health(_))
                || record
                    .event
                    .feeds()
                    .iter()
                    .any(|feed| feed.state != BookState::Valid);
            last_event_at_ms = Some(observed_at_ms);
            records += 1;

            for feed in record.event.feeds() {
                run_reconnects
                    .entry((record.run_id.clone(), feed.venue.clone()))
                    .and_modify(|count: &mut u64| *count = (*count).max(feed.reconnects))
                    .or_insert(feed.reconnects);
            }
            match &record.event {
                ArchivedEvent::Decision(event) => {
                    decision_records += 1;
                    let replayed = event
                        .replay()
                        .with_context(|| format!("failed to replay {}", record.event_id))?;
                    if replayed != event.report {
                        bail!("replay mismatch for {}", record.event_id);
                    }
                    capacities.extend(visible_direction_capacities(&event.books));
                    for opportunity in &event.report.directions {
                        profits.push(opportunity.expected_net_profit);
                        if opportunity.expected_net_profit > Decimal::ZERO {
                            positive_net_opportunities += 1;
                        }
                        if opportunity.accepted {
                            accepted_opportunities += 1;
                        }
                        for reason in &opportunity.rejection_reasons {
                            increment_rejection_count(&mut rejection_reason_counts, reason);
                        }
                        tails.push(TailSample {
                            event_id: record.event_id.clone(),
                            buy_venue: opportunity.buy_venue.clone(),
                            sell_venue: opportunity.sell_venue.clone(),
                            admission_profit: opportunity.admission_profit,
                            admission_net_bps: opportunity.admission_net_bps,
                        });
                        tails.sort_by_key(|sample| sample.admission_profit);
                        tails.truncate(TAIL_SAMPLE_LIMIT);
                    }
                }
                ArchivedEvent::Health(_) => health_records += 1,
            }
            Ok(())
        })?;

    profits.sort();
    capacities.sort();
    let visible_capacity = capacity_distribution(&capacities);
    for ((_, venue), count) in run_reconnects {
        *reconnects.entry(venue).or_default() += count;
    }
    let distribution = distribution(&profits);
    let observed_duration_ms = first_event_at_ms
        .zip(last_event_at_ms)
        .map_or(0, |(first, last)| last.saturating_sub(first));
    let (gap_records, dropped_events, gap_snapshot) = read_gap_totals(gap_path.as_ref())?;
    Ok(ShadowReport {
        schema_version: SCHEMA_VERSION,
        archive_path: path.to_owned(),
        archive_bytes: snapshot.bytes,
        ignored_incomplete_tail_bytes: snapshot.ignored_tail_bytes,
        records,
        decision_records,
        health_records,
        first_event_at_ms,
        last_event_at_ms,
        observed_duration_ms,
        online_duration_ms: observed_duration_ms.saturating_sub(invalid_duration_ms),
        invalid_duration_ms,
        reconnects,
        direction_evaluations: profits.len() as u64,
        positive_net_opportunities,
        accepted_opportunities,
        net_profit_distribution: distribution,
        visible_capacity,
        rejection_reason_counts,
        tail_samples: tails,
        gap_records,
        dropped_events,
        ignored_gap_tail_bytes: gap_snapshot.ignored_tail_bytes,
        replayed_without_mismatch: true,
    })
}

#[derive(Debug, Default)]
struct SnapshotRead {
    bytes: u64,
    ignored_tail_bytes: u64,
}

fn for_each_snapshot_record<T, F>(
    path: &Path,
    label: &str,
    missing_is_empty: bool,
    mut visit: F,
) -> Result<SnapshotRead>
where
    T: for<'de> Deserialize<'de>,
    F: FnMut(T) -> Result<()>,
{
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if missing_is_empty && error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SnapshotRead::default());
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to open {} {}", label, path.display()));
        }
    };
    let bytes = file
        .metadata()
        .with_context(|| format!("failed to inspect {} {}", label, path.display()))?
        .len();
    let mut reader = BufReader::new(file.take(bytes));
    let mut line = Vec::new();
    let mut line_number = 0_usize;
    let mut ignored_tail_bytes = 0_u64;
    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .with_context(|| format!("failed to read {} line {}", label, line_number + 1))?;
        if read == 0 {
            break;
        }
        line_number += 1;
        if !line.ends_with(b"\n") {
            ignored_tail_bytes = read as u64;
            break;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let record = serde_json::from_slice(&line)
            .with_context(|| format!("invalid {} at line {}", label, line_number))?;
        visit(record)?;
    }
    Ok(SnapshotRead {
        bytes,
        ignored_tail_bytes,
    })
}

fn increment_rejection_count(counts: &mut BTreeMap<String, u64>, reason: &str) {
    *counts.entry(rejection_reason_category(reason)).or_default() += 1;
}

fn rejection_reason_category(reason: &str) -> String {
    for (prefix, category) in [
        ("admission profit ", "admission_profit_below_minimum"),
        ("admission net bps ", "admission_net_bps_below_minimum"),
        ("first snapshot age ", "first_snapshot_stale"),
        ("second snapshot age ", "second_snapshot_stale"),
        ("snapshot receive skew ", "snapshot_receive_skew"),
    ] {
        if reason.starts_with(prefix) {
            return category.to_owned();
        }
    }
    for venue in ["binance", "bybit"] {
        let Some(detail) = reason
            .strip_prefix(venue)
            .and_then(|remaining| remaining.strip_prefix(' '))
        else {
            continue;
        };
        let category = if detail.starts_with("account metadata refresh failed:") {
            "account_metadata_refresh_failed"
        } else if detail.starts_with("API key has forbidden permissions:") {
            "forbidden_api_permissions"
        } else if detail.contains(" quantity ") || detail.starts_with("quantity ") {
            "quantity_filter"
        } else if detail.contains(" notional ") || detail.starts_with("notional ") {
            "notional_filter"
        } else if detail == "region eligibility is not confirmed" {
            "region_eligibility_unconfirmed"
        } else if detail == "account eligibility is not confirmed" {
            "account_eligibility_unconfirmed"
        } else if detail == "read-only credentials are not configured" {
            "credentials_not_configured"
        } else if detail
            == "actual account fee unavailable; configured fallback is observation-only"
        {
            "actual_fee_unavailable"
        } else if detail == "actual account fee is expired" {
            "actual_fee_expired"
        } else if detail == "API key is not read-only" {
            "api_key_not_read_only"
        } else if detail == "API key has forbidden withdrawal permission" {
            "forbidden_withdrawal_permission"
        } else {
            return "other_rejection_reason".to_owned();
        };
        return format!("{venue}:{category}");
    }
    "other_rejection_reason".to_owned()
}

#[cfg(test)]
fn rejection_counts(reasons: impl IntoIterator<Item = String>) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for reason in reasons {
        increment_rejection_count(&mut counts, &reason);
    }
    counts
}
fn read_gap_totals(path: &Path) -> Result<(u64, u64, SnapshotRead)> {
    let mut records = 0_u64;
    let mut dropped = 0_u64;
    let snapshot = for_each_snapshot_record(path, "archive gap", true, |gap: ArchiveGap| {
        records += 1;
        dropped = dropped.saturating_add(gap.dropped_events);
        Ok(())
    })?;
    Ok((records, dropped, snapshot))
}

fn distribution(sorted: &[Decimal]) -> Option<ProfitDistribution> {
    if sorted.is_empty() {
        return None;
    }
    let percentile = |numerator: usize, denominator: usize| {
        let index = (sorted.len() - 1) * numerator / denominator;
        sorted[index]
    };
    Some(ProfitDistribution {
        minimum: sorted[0],
        p50: percentile(50, 100),
        p95: percentile(95, 100),
        maximum: sorted[sorted.len() - 1],
    })
}

fn visible_direction_capacities(books: &[OrderBookSnapshot; 2]) -> [Decimal; 2] {
    let first_bids: Decimal = books[0].bids.iter().map(|level| level.quantity).sum();
    let first_asks: Decimal = books[0].asks.iter().map(|level| level.quantity).sum();
    let second_bids: Decimal = books[1].bids.iter().map(|level| level.quantity).sum();
    let second_asks: Decimal = books[1].asks.iter().map(|level| level.quantity).sum();
    [first_asks.min(second_bids), second_asks.min(first_bids)]
}

fn capacity_distribution(sorted: &[Decimal]) -> Option<CapacityDistribution> {
    if sorted.is_empty() {
        return None;
    }
    let percentile = |numerator: usize, denominator: usize| {
        let index = (sorted.len() - 1) * numerator / denominator;
        sorted[index]
    };
    Some(CapacityDistribution {
        minimum_base_quantity: sorted[0],
        p50_base_quantity: percentile(50, 100),
        p95_base_quantity: percentile(95, 100),
        maximum_base_quantity: sorted[sorted.len() - 1],
    })
}

pub fn default_gap_path(archive_path: &Path) -> PathBuf {
    let mut name = archive_path
        .file_name()
        .map_or_else(|| "archive".into(), |name| name.to_os_string());
    name.push(".gaps.ndjson");
    archive_path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        instrument::{OrderCapability, TradingStatus},
        market::Level,
    };

    fn dec(value: &str) -> Decimal {
        value.parse().unwrap()
    }

    fn temp_paths(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "personal-taoli-{name}-{}-{}",
            std::process::id(),
            RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&root).unwrap();
        let archive = root.join("observations.ndjson");
        let gaps = default_gap_path(&archive);
        (root, archive, gaps)
    }

    fn book(
        venue: &str,
        bid: &str,
        bid_quantity: &str,
        ask: &str,
        ask_quantity: &str,
        at: u64,
    ) -> OrderBookSnapshot {
        OrderBookSnapshot {
            venue: venue.into(),
            symbol: "BTCUSDT".into(),
            bids: vec![Level {
                price: dec(bid),
                quantity: dec(bid_quantity),
            }],
            asks: vec![Level {
                price: dec(ask),
                quantity: dec(ask_quantity),
            }],
            sequence: at,
            source_timestamp_ms: Some(at),
            received_timestamp_ms: at,
        }
    }

    fn feed(venue: &str, state: BookState, reconnects: u64) -> FeedVersion {
        FeedVersion {
            venue: venue.into(),
            symbol: "BTCUSDT".into(),
            state,
            generation: reconnects + 1,
            reconnects,
            applied_updates: 10,
            reason: (state != BookState::Valid).then(|| "test feed unavailable".into()),
        }
    }

    fn instrument(venue: &str) -> InstrumentSpec {
        InstrumentSpec {
            venue: venue.into(),
            account_type: "spot".into(),
            venue_symbol: "BTCUSDT".into(),
            canonical_instrument_id: "BTC/USDT:SPOT".into(),
            base_asset_id: "BTC".into(),
            quote_asset_id: "USDT".into(),
            settlement_asset_id: "USDT".into(),
            market_type: "spot".into(),
            contract_multiplier: Decimal::ONE,
            quantity_unit: "base_asset".into(),
            price_tick: dec("0.01"),
            quantity_step: dec("0.001"),
            min_quantity: dec("0.001"),
            max_quantity: Some(dec("100")),
            min_notional: Decimal::ZERO,
            max_notional: None,
            trading_status: TradingStatus::Trading,
            supported_order_types: vec![OrderCapability::Limit, OrderCapability::Ioc],
            metadata_version: 7,
        }
    }

    fn decision(at: u64, reconnects: [u64; 2]) -> DecisionEvent {
        let books = [
            book("binance", "99", "5", "100", "4", at),
            book("bybit", "110", "3", "111", "2", at),
        ];
        let instruments = [instrument("binance"), instrument("bybit")];
        let strategy = StrategyConfig {
            min_net_profit: Decimal::ZERO,
            min_net_bps: Decimal::ZERO,
            latency_loss_bps: Decimal::ZERO,
            risk_buffer_bps: Decimal::ZERO,
            rebalance_cost: Decimal::ZERO,
            other_direct_cost: Decimal::ZERO,
        };
        let config = DecisionConfig {
            symbol: "BTCUSDT".into(),
            base_asset: "BTC".into(),
            quote_asset: "USDT".into(),
            quantity: Decimal::ONE,
            strategy,
            freshness: FreshnessLimits {
                max_snapshot_age_ms: 1_000,
                max_pair_skew_ms: 500,
            },
        };
        let fees = [
            ArchivedFeeVersion {
                venue: "binance".into(),
                symbol: "BTCUSDT".into(),
                buy_taker_rate: Decimal::ZERO,
                sell_taker_rate: Decimal::ZERO,
                source: "test".into(),
                loaded_at_ms: Some(at),
                expires_at_ms: Some(at + 1_000),
                actual_account_rate: true,
            },
            ArchivedFeeVersion {
                venue: "bybit".into(),
                symbol: "BTCUSDT".into(),
                buy_taker_rate: Decimal::ZERO,
                sell_taker_rate: Decimal::ZERO,
                source: "test".into(),
                loaded_at_ms: Some(at),
                expires_at_ms: Some(at + 1_000),
                actual_account_rate: true,
            },
        ];
        let report = scan_pair(ScanInput {
            first_book: &books[0],
            second_book: &books[1],
            instruments: InstrumentPair {
                first: &instruments[0],
                second: &instruments[1],
            },
            quantity: config.quantity,
            fees: VenueFees {
                first: VenueFeeRates {
                    buy_taker_rate: fees[0].buy_taker_rate,
                    sell_taker_rate: fees[0].sell_taker_rate,
                },
                second: VenueFeeRates {
                    buy_taker_rate: fees[1].buy_taker_rate,
                    sell_taker_rate: fees[1].sell_taker_rate,
                },
            },
            strategy: &config.strategy,
            now_ms: at,
            freshness: config.freshness,
            admission_rejections: &[],
        })
        .unwrap();
        DecisionEvent {
            evaluated_at_ms: at,
            feeds: [
                feed("binance", BookState::Valid, reconnects[0]),
                feed("bybit", BookState::Valid, reconnects[1]),
            ],
            books,
            instruments,
            fees,
            config_version: content_version(&config).unwrap(),
            config,
            admission_rejections: Vec::new(),
            report,
        }
    }

    #[test]
    fn archives_replay_exactly_and_report_online_invalid_and_capacity_metrics() {
        let (root, archive, gaps) = temp_paths("archive-replay");
        let mut writer = ArchiveWriter::start(&archive, &gaps, 8, 30).unwrap();
        writer
            .submit(ArchivedEvent::Decision(Box::new(decision(1_000, [2, 1]))))
            .unwrap();
        writer
            .submit(ArchivedEvent::Health(Box::new(HealthEvent {
                observed_at_ms: 1_100,
                feeds: [
                    feed("binance", BookState::Stale, 3),
                    feed("bybit", BookState::Valid, 1),
                ],
                skip_reason: "binance stale".into(),
            })))
            .unwrap();
        writer
            .submit(ArchivedEvent::Decision(Box::new(decision(1_200, [3, 1]))))
            .unwrap();
        writer.finish().unwrap();

        let report = replay_archive(&archive, &gaps).unwrap();
        assert!(report.replayed_without_mismatch);
        assert_eq!(
            (
                report.records,
                report.decision_records,
                report.health_records
            ),
            (3, 2, 1)
        );
        assert_eq!(
            (
                report.observed_duration_ms,
                report.online_duration_ms,
                report.invalid_duration_ms
            ),
            (200, 100, 100)
        );
        assert_eq!(
            (
                report.direction_evaluations,
                report.positive_net_opportunities,
                report.accepted_opportunities
            ),
            (4, 2, 2)
        );
        assert_eq!(report.reconnects.get("binance"), Some(&3));
        assert_eq!(report.reconnects.get("bybit"), Some(&1));
        assert_eq!(
            report.net_profit_distribution.as_ref().unwrap().minimum,
            dec("-12")
        );
        assert_eq!(
            report.net_profit_distribution.as_ref().unwrap().maximum,
            dec("10")
        );
        let capacity = report.visible_capacity.unwrap();
        assert_eq!(capacity.minimum_base_quantity, dec("2"));
        assert_eq!(capacity.maximum_base_quantity, dec("3"));
        assert_eq!((report.gap_records, report.dropped_events), (0, 0));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replay_rejects_a_decision_whose_saved_output_was_tampered() {
        let (root, archive, gaps) = temp_paths("archive-mismatch");
        let mut event = decision(1_000, [0, 0]);
        event.report.directions[0].expected_net_profit += Decimal::ONE;
        let record =
            ArchiveRecord::new("tampered", 1, 30, ArchivedEvent::Decision(Box::new(event)))
                .unwrap();
        fs::write(
            &archive,
            format!("{}\n", serde_json::to_string(&record).unwrap()),
        )
        .unwrap();

        let error = replay_archive(&archive, &gaps).unwrap_err();
        assert!(error.to_string().contains("replay mismatch"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn archive_io_failure_is_recorded_as_a_gap_without_failing_submission() {
        let (root, archive, gaps) = temp_paths("archive-io-failure");
        fs::create_dir(&archive).unwrap();
        let mut writer = ArchiveWriter::start(&archive, &gaps, 8, 30).unwrap();
        writer
            .submit(ArchivedEvent::Health(Box::new(HealthEvent {
                observed_at_ms: 1_000,
                feeds: [
                    feed("binance", BookState::Invalid, 0),
                    feed("bybit", BookState::Valid, 0),
                ],
                skip_reason: "archive failure test".into(),
            })))
            .unwrap();
        writer.finish().unwrap();

        assert_eq!(read_gap_totals(&gaps).unwrap().0, 1);
        assert_eq!(read_gap_totals(&gaps).unwrap().1, 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replay_uses_complete_snapshot_while_last_line_is_still_being_written() {
        let (root, archive, gaps) = temp_paths("archive-live-tail");
        let record = ArchiveRecord::new(
            "live",
            1,
            30,
            ArchivedEvent::Decision(Box::new(decision(1_000, [0, 0]))),
        )
        .unwrap();
        fs::write(
            &archive,
            format!("{}\n{{\"partial\"", serde_json::to_string(&record).unwrap()),
        )
        .unwrap();

        let report = replay_archive(&archive, &gaps).unwrap();
        assert_eq!(report.records, 1);
        assert_eq!(report.ignored_incomplete_tail_bytes, 10);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restart_repairs_incomplete_tail_and_audits_discarded_bytes() {
        let (root, archive, gaps) = temp_paths("archive-tail-repair");
        let first = ArchiveRecord::new(
            "before-crash",
            1,
            30,
            ArchivedEvent::Decision(Box::new(decision(1_000, [0, 0]))),
        )
        .unwrap();
        fs::write(
            &archive,
            format!("{}\npartial", serde_json::to_string(&first).unwrap()),
        )
        .unwrap();

        let mut writer = ArchiveWriter::start(&archive, &gaps, 8, 30).unwrap();
        writer
            .submit(ArchivedEvent::Decision(Box::new(decision(1_100, [0, 0]))))
            .unwrap();
        writer.finish().unwrap();

        let report = replay_archive(&archive, &gaps).unwrap();
        assert_eq!(report.records, 2);
        assert_eq!(report.ignored_incomplete_tail_bytes, 0);
        assert_eq!((report.gap_records, report.dropped_events), (1, 0));
        let gap_text = fs::read_to_string(&gaps).unwrap();
        assert!(gap_text.contains("discarded 7 incomplete bytes from archive"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replay_rejects_newline_terminated_corruption() {
        let (root, archive, gaps) = temp_paths("archive-corrupt-line");
        fs::write(&archive, "not-json\n").unwrap();

        let error = replay_archive(&archive, &gaps).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("invalid archive record at line 1")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replay_rejects_event_time_regression() {
        let (root, archive, gaps) = temp_paths("archive-time-regression");
        let records = [
            ArchiveRecord::new(
                "ordered",
                1,
                30,
                ArchivedEvent::Decision(Box::new(decision(1_100, [0, 0]))),
            )
            .unwrap(),
            ArchiveRecord::new(
                "ordered",
                2,
                30,
                ArchivedEvent::Decision(Box::new(decision(1_000, [0, 0]))),
            )
            .unwrap(),
        ];
        fs::write(
            &archive,
            records
                .iter()
                .map(|record| format!("{}\n", serde_json::to_string(record).unwrap()))
                .collect::<String>(),
        )
        .unwrap();

        let error = replay_archive(&archive, &gaps).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("archive event time moved backward")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejection_report_normalizes_dynamic_errors_and_bounds_unknown_categories() {
        let reasons = (0..200)
            .map(|index| format!("unknown dynamic rejection {index}"))
            .chain([
                "binance account metadata refresh failed: HTTP 429 request 1".to_owned(),
                "binance account metadata refresh failed: HTTP 500 request 2".to_owned(),
            ]);

        let counts = rejection_counts(reasons);
        assert_eq!(counts.len(), 2);
        assert_eq!(counts.get("other_rejection_reason"), Some(&200));
        assert_eq!(
            counts.get("binance:account_metadata_refresh_failed"),
            Some(&2)
        );
    }
}
