//! Rust port of the UZI-Skill analysis core.
//!
//! Only the deterministic tail of the original Python pipeline is ported:
//! scoring, investor rules, synthesis and rendering. Data collection stays in
//! Python because it is built on `akshare` / `yfinance` / `baostock` / `ddgs`,
//! which have no Rust equivalents — see the iteration notes for the rationale.
//!
//! Ports are validated by differential comparison against reference artifacts
//! produced by the original code; see `examples/score_diff.rs`. That is why
//! `py.rs` reproduces CPython semantics rather than using idiomatic Rust.

pub mod evaluator;
pub mod features;
pub mod panel;
pub mod panel_data;
pub mod pipeline;
pub mod py;
pub mod report;
pub mod rules;
pub mod score;
pub mod style;
pub mod synthesis;

pub use rules::{Rule, RuleSet, eval};
pub use score::score_dimensions;

/// Schema version this build expects from `tools/compile_rules.py`.
pub const SPEC_VERSION: i64 = 1;

/// Compiled investor rule specs (180 rules / 43 groups).
pub const RULES_JSON: &str = include_str!("../tools/out/rules.json");

/// Investor metadata (66 records).
pub const INVESTORS_JSON: &str = include_str!("../tools/out/investors.json");

/// Static panel tables (personas, profiles, knowledge, feature keywords).
pub const PANEL_DATA_JSON: &str = include_str!("../tools/out/panel_data.json");

/// Parsed panel tables.
pub fn panel_data() -> anyhow::Result<panel_data::PanelData> {
    panel_data::PanelData::parse(PANEL_DATA_JSON)
}

/// The investor database as the original Python module defines it.
pub fn investors() -> anyhow::Result<Vec<serde_json::Value>> {
    let doc: serde_json::Value = serde_json::from_str(INVESTORS_JSON)
        .map_err(|e| anyhow::anyhow!("embedded investors.json is invalid: {e}"))?;
    if doc.get("schema").and_then(|v| v.as_i64()) != Some(SPEC_VERSION) {
        anyhow::bail!("embedded investors.json has a stale schema; re-run tools/compile_rules.py");
    }
    Ok(doc
        .get("investors")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default())
}

/// The compiled rule set.
pub fn rules() -> anyhow::Result<RuleSet> {
    RuleSet::parse(RULES_JSON)
}
