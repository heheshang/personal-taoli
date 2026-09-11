//! Read-only tool exposing the continuous shadow statistics.
//!
//! The data path is the existing core read path — [`replay_archive`] over the
//! same NDJSON archive the observer writes and `replay_observations` re-reads.
//! Nothing is recomputed here, so a value reported to the model cannot drift
//! from a value reported to the operator.
//!
//! Freshness is enforced, not reported: if the newest archived event is older
//! than the configured tolerance the tool **fails closed** rather than handing
//! back stale observations as if they were current. That mirrors the observer's
//! own rule (a scan stops when its snapshots age out) and the project's
//! invariant that invalid data must not silently become a decision input.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::archive::{ShadowReport, default_gap_path, replay_archive};
use crate::market::unix_timestamp_ms;

use super::tools::{AgentTool, ToolOutcome};

/// Tool name as seen by the model.
pub const TOOL_NAME: &str = "taoli_shadow_report";

/// Read-only view of the shadow archive.
pub struct ShadowReportTool {
    archive_path: PathBuf,
    gap_path: PathBuf,
    /// Oldest acceptable age of the newest archived event.
    ///
    /// Host policy, not model input: freshness is a property of the deployment,
    /// so the model cannot widen it by asking.
    max_observation_age_ms: u64,
}

impl ShadowReportTool {
    /// Builds the tool for `archive_path`, deriving the gap path the same way
    /// the observer does.
    pub fn new(archive_path: impl Into<PathBuf>, max_observation_age_ms: u64) -> Self {
        let archive_path = archive_path.into();
        let gap_path = default_gap_path(&archive_path);
        Self {
            archive_path,
            gap_path,
            max_observation_age_ms,
        }
    }

    pub fn archive_path(&self) -> &Path {
        &self.archive_path
    }

    pub fn max_observation_age_ms(&self) -> u64 {
        self.max_observation_age_ms
    }

    /// Summarises a replayed report for the model.
    ///
    /// Kept separate from [`AgentTool::call`] so the mapping from the core read
    /// path to the tool payload is directly testable against a report the test
    /// obtained itself.
    pub fn summarise(report: &ShadowReport, now_ms: u64) -> Summary {
        Summary {
            schema_version: report.schema_version,
            records: report.records,
            decision_records: report.decision_records,
            health_records: report.health_records,
            first_event_at_ms: report.first_event_at_ms,
            last_event_at_ms: report.last_event_at_ms,
            observation_age_ms: report
                .last_event_at_ms
                .map(|last| now_ms.saturating_sub(last)),
            observed_duration_ms: report.observed_duration_ms,
            online_duration_ms: report.online_duration_ms,
            invalid_duration_ms: report.invalid_duration_ms,
            reconnects: report.reconnects.clone(),
            direction_evaluations: report.direction_evaluations,
            positive_net_opportunities: report.positive_net_opportunities,
            accepted_opportunities: report.accepted_opportunities,
            net_profit: report
                .net_profit_distribution
                .as_ref()
                .map(|d| Distribution {
                    minimum: d.minimum.to_string(),
                    p50: d.p50.to_string(),
                    p95: d.p95.to_string(),
                    maximum: d.maximum.to_string(),
                }),
            visible_capacity: report.visible_capacity.as_ref().map(|c| Capacity {
                minimum_base_quantity: c.minimum_base_quantity.to_string(),
                p50_base_quantity: c.p50_base_quantity.to_string(),
                p95_base_quantity: c.p95_base_quantity.to_string(),
                maximum_base_quantity: c.maximum_base_quantity.to_string(),
            }),
            rejection_reason_counts: report.rejection_reason_counts.clone(),
            gap_records: report.gap_records,
            dropped_events: report.dropped_events,
            replayed_without_mismatch: report.replayed_without_mismatch,
        }
    }
}

/// Model-facing summary of [`ShadowReport`].
///
/// Amounts stay as decimal strings, matching the project-wide rule that
/// monetary values never become floats.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Summary {
    pub schema_version: u16,
    pub records: u64,
    pub decision_records: u64,
    pub health_records: u64,
    pub first_event_at_ms: Option<u64>,
    pub last_event_at_ms: Option<u64>,
    pub observation_age_ms: Option<u64>,
    pub observed_duration_ms: u64,
    pub online_duration_ms: u64,
    pub invalid_duration_ms: u64,
    pub reconnects: std::collections::BTreeMap<String, u64>,
    pub direction_evaluations: u64,
    pub positive_net_opportunities: u64,
    pub accepted_opportunities: u64,
    pub net_profit: Option<Distribution>,
    pub visible_capacity: Option<Capacity>,
    pub rejection_reason_counts: std::collections::BTreeMap<String, u64>,
    pub gap_records: u64,
    pub dropped_events: u64,
    pub replayed_without_mismatch: bool,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Distribution {
    pub minimum: String,
    pub p50: String,
    pub p95: String,
    pub maximum: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Capacity {
    pub minimum_base_quantity: String,
    pub p50_base_quantity: String,
    pub p95_base_quantity: String,
    pub maximum_base_quantity: String,
}

impl AgentTool for ShadowReportTool {
    fn name(&self) -> &'static str {
        TOOL_NAME
    }

    fn description(&self) -> &'static str {
        "Replay the Taoli observation archive and return the continuous shadow \
         statistics (record counts, feed reconnects, opportunity counts, net \
         profit and visible capacity distributions, rejection reasons). \
         Read-only. Fails if the archive is missing or its newest event is \
         older than the configured freshness tolerance."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false,
        })
    }

    fn call(&self, _arguments: &serde_json::Value) -> ToolOutcome {
        // The schema declares no arguments, but a model can still send extras.
        // Ignoring them is safe here because none of them can widen the read or
        // relax the freshness gate.
        let report = match replay_archive(&self.archive_path, &self.gap_path) {
            Ok(report) => report,
            Err(error) => {
                return ToolOutcome::failed(format!(
                    "cannot replay archive {}: {error:#}",
                    self.archive_path.display()
                ));
            }
        };

        let now_ms = match unix_timestamp_ms() {
            Ok(now_ms) => now_ms,
            Err(error) => return ToolOutcome::failed(format!("cannot read clock: {error:#}")),
        };
        let summary = Self::summarise(&report, now_ms);

        // Fail closed on stale data instead of returning it.
        match summary.observation_age_ms {
            None => ToolOutcome::failed(format!(
                "archive {} contains no events; nothing current to report",
                self.archive_path.display()
            )),
            Some(age) if age > self.max_observation_age_ms => ToolOutcome::failed(format!(
                "refusing to report stale observations: newest archived event is {age} ms old, \
                 tolerance is {} ms (archive {})",
                self.max_observation_age_ms,
                self.archive_path.display()
            )),
            Some(_) => match serde_json::to_string(&summary) {
                Ok(json) => ToolOutcome::text(json),
                Err(error) => ToolOutcome::failed(format!("cannot encode summary: {error}")),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::replay_archive;

    /// The observer archive, used as the real read path fixture.
    ///
    /// `data/` is gitignored local state, so a fresh clone has no fixture and
    /// the tests that need it skip rather than fail.
    fn repo_archive() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/archive/observations.ndjson")
    }

    fn tool_with_tolerance(max_observation_age_ms: u64) -> ShadowReportTool {
        ShadowReportTool::new(repo_archive(), max_observation_age_ms)
    }

    /// Acceptance: the payload handed to the model must equal what a direct
    /// call to the core read path produces. Nothing is recomputed in between.
    #[test]
    fn summary_matches_a_direct_replay_of_the_same_read_path() {
        let path = repo_archive();
        if !path.is_file() {
            eprintln!("skipping: {} is absent", path.display());
            return;
        }

        // Direct call to the core read path, by the test itself.
        let direct = replay_archive(&path, default_gap_path(&path)).expect("direct replay");
        let now_ms = unix_timestamp_ms().expect("clock");

        let summary = ShadowReportTool::summarise(&direct, now_ms);
        assert_eq!(summary.records, direct.records);
        assert_eq!(summary.decision_records, direct.decision_records);
        assert_eq!(summary.health_records, direct.health_records);
        assert_eq!(summary.last_event_at_ms, direct.last_event_at_ms);
        assert_eq!(
            summary.positive_net_opportunities,
            direct.positive_net_opportunities
        );
        assert_eq!(
            summary.accepted_opportunities,
            direct.accepted_opportunities
        );
        assert_eq!(summary.reconnects, direct.reconnects);

        // And the tool must emit exactly that, when the data is fresh enough.
        let tool = ShadowReportTool::new(&path, u64::MAX);
        let outcome = tool.call(&serde_json::json!({}));
        assert!(outcome.success, "fresh-enough archive must be reported");
        let emitted: Summary = match &outcome.content_items[0] {
            super::super::tools::ContentItem::Text(text) => {
                serde_json::from_str(text).expect("payload is the summary JSON")
            }
        };

        // `observation_age_ms` is derived from the wall clock, so two reads
        // cannot agree exactly. Every other field must be identical, and the
        // tool's age must be the real elapsed time since the newest event —
        // not merely *some* number.
        let expected_age = summary.observation_age_ms.expect("archive has events");
        let emitted_age = emitted.observation_age_ms.expect("archive has events");
        assert!(
            emitted_age >= expected_age,
            "age must not go backwards: {emitted_age} < {expected_age}"
        );
        assert!(
            emitted_age - expected_age < 5_000,
            "two clock reads drifted by {} ms",
            emitted_age - expected_age
        );

        let mut expected = summary;
        expected.observation_age_ms = emitted.observation_age_ms;
        assert_eq!(emitted, expected);
    }

    /// Acceptance: stale data must be refused, not returned.
    #[test]
    fn stale_archive_fails_closed_instead_of_reporting_old_data() {
        let path = repo_archive();
        if !path.is_file() {
            eprintln!("skipping: {} is absent", path.display());
            return;
        }

        // Zero tolerance: any measurable age exceeds it.
        let tool = tool_with_tolerance(0);
        let outcome = tool.call(&serde_json::json!({}));
        assert!(!outcome.success, "stale data must not be reported");
        let super::super::tools::ContentItem::Text(text) = &outcome.content_items[0];
        assert!(
            text.contains("refusing to report stale observations"),
            "{text}"
        );
        // The message must name both sides of the comparison.
        assert!(text.contains("tolerance is 0 ms"), "{text}");
    }

    #[test]
    fn missing_archive_fails_closed_with_the_path() {
        let tool = ShadowReportTool::new("/nonexistent/observations.ndjson", u64::MAX);
        let outcome = tool.call(&serde_json::json!({}));
        assert!(!outcome.success);
        let super::super::tools::ContentItem::Text(text) = &outcome.content_items[0];
        assert!(text.contains("cannot replay archive"), "{text}");
        assert!(text.contains("/nonexistent/observations.ndjson"), "{text}");
    }

    #[test]
    fn gap_path_is_derived_the_same_way_the_observer_derives_it() {
        let tool = ShadowReportTool::new("/tmp/x/observations.ndjson", 1);
        assert_eq!(tool.archive_path(), Path::new("/tmp/x/observations.ndjson"));
        assert_eq!(
            tool.gap_path,
            Path::new("/tmp/x/observations.ndjson.gaps.ndjson")
        );
    }

    #[test]
    fn schema_declares_no_arguments() {
        let schema = tool_with_tolerance(1).input_schema();
        assert_eq!(schema["type"], serde_json::json!("object"));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert!(schema["required"].as_array().expect("array").is_empty());
    }
}
