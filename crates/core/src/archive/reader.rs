//! 归档读取与回放。

use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
};

use anyhow::{Context, Result, bail};
use rust_decimal::Decimal;

use crate::{local_book::BookState, market::OrderBookSnapshot};

use super::{
    SCHEMA_VERSION,
    event_types::{ArchiveGap, ArchiveRecord, ArchivedEvent},
    report::ShadowReport,
};

#[derive(Debug, Default)]
pub(crate) struct SnapshotRead {
    pub bytes: u64,
    pub ignored_tail_bytes: u64,
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
                        tails.push(super::report::TailSample {
                            event_id: record.event_id.clone(),
                            buy_venue: opportunity.buy_venue.clone(),
                            sell_venue: opportunity.sell_venue.clone(),
                            admission_profit: opportunity.admission_profit,
                            admission_net_bps: opportunity.admission_net_bps,
                        });
                        tails.sort_by_key(|sample| sample.admission_profit);
                        tails.truncate(super::report::TAIL_SAMPLE_LIMIT);
                    }
                }
                ArchivedEvent::Health(_) => health_records += 1,
            }
            Ok(())
        })?;

    profits.sort();
    capacities.sort();
    let visible_capacity = super::report::capacity_distribution(&capacities);
    for ((_, venue), count) in run_reconnects {
        *reconnects.entry(venue).or_default() += count;
    }
    let distribution = super::report::distribution(&profits);
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

pub(crate) fn for_each_snapshot_record<T, F>(
    path: &Path,
    label: &str,
    missing_is_empty: bool,
    mut visit: F,
) -> Result<SnapshotRead>
where
    T: for<'de> serde::Deserialize<'de>,
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

fn visible_direction_capacities(books: &[OrderBookSnapshot; 2]) -> [Decimal; 2] {
    let first_bids: Decimal = books[0].bids.iter().map(|level| level.quantity).sum();
    let first_asks: Decimal = books[0].asks.iter().map(|level| level.quantity).sum();
    let second_bids: Decimal = books[1].bids.iter().map(|level| level.quantity).sum();
    let second_asks: Decimal = books[1].asks.iter().map(|level| level.quantity).sum();
    [first_asks.min(second_bids), second_asks.min(first_bids)]
}
