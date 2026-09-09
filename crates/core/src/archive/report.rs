//! 归档报告类型与统计函数。

use std::{collections::BTreeMap, path::PathBuf};

use rust_decimal::Decimal;
use serde::Serialize;

pub(crate) const TAIL_SAMPLE_LIMIT: usize = 5;

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

pub(crate) fn percentile(sorted: &[Decimal], numerator: usize, denominator: usize) -> Decimal {
    let index = (sorted.len() - 1) * numerator / denominator;
    sorted[index]
}

pub(crate) fn distribution(sorted: &[Decimal]) -> Option<ProfitDistribution> {
    if sorted.is_empty() {
        return None;
    }
    Some(ProfitDistribution {
        minimum: sorted[0],
        p50: percentile(sorted, 50, 100),
        p95: percentile(sorted, 95, 100),
        maximum: sorted[sorted.len() - 1],
    })
}

pub(crate) fn capacity_distribution(sorted: &[Decimal]) -> Option<CapacityDistribution> {
    if sorted.is_empty() {
        return None;
    }
    Some(CapacityDistribution {
        minimum_base_quantity: sorted[0],
        p50_base_quantity: percentile(sorted, 50, 100),
        p95_base_quantity: percentile(sorted, 95, 100),
        maximum_base_quantity: sorted[sorted.len() - 1],
    })
}

pub fn default_gap_path(archive_path: &std::path::Path) -> PathBuf {
    let mut name = archive_path
        .file_name()
        .map_or_else(|| "archive".into(), |name| name.to_os_string());
    name.push(".gaps.ndjson");
    archive_path.with_file_name(name)
}
