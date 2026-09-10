//! Port of `stock_style` (UZI-Skill v3.9.4).
//!
//! Stock-style detection and the style-weighted re-scoring. The original notes
//! the motive: a naive equal-weight panel produced "almost everything is avoid",
//! so schools get style-dependent weights and individual investors can override
//! their school's weight.
//!
//! `detect_style`'s quant-factor branch calls `detect_quant_signal`, which
//! fetches fund holdings over the network (695 cached files in the reference
//! run). That is data collection, not analysis, so it enters as a parameter —
//! the same seam as the persona comment.

use crate::panel_data::StyleData;
use crate::py::round_to;
use serde_json::Value;

pub const WHITE_HORSE: &str = "white_horse";
pub const GROWTH_TECH: &str = "growth_tech";
pub const CYCLE: &str = "cycle";
pub const SMALL_SPECULATIVE: &str = "small_speculative";
pub const DIVIDEND_DEFENSE: &str = "dividend_defense";
pub const DISTRESSED: &str = "distressed";
pub const QUANT_FACTOR: &str = "quant_factor";
pub const BALANCED: &str = "balanced";

/// `stock_style._f` — same cleaning as `stock_features._f`.
fn f(v: &Value, default: f64) -> f64 {
    crate::features::f(v, Some(default)).unwrap_or(default)
}

fn s(v: &Value) -> String {
    crate::py::py_str(v)
}

/// Detects the stock's style.
///
/// `is_quant_factor_style` is supplied by the caller: in the original it comes
/// from `detect_quant_signal`, which fetches fund top-10 holdings. Rules are
/// evaluated in the original's order, first match wins.
pub fn detect_style(style: &StyleData, features: &Value, is_quant_factor_style: bool) -> String {
    let pb = f(crate::py::get(features, "pb"), 0.0);
    let pe = {
        let a = f(crate::py::get(features, "pe"), 0.0);
        if a != 0.0 {
            a
        } else {
            f(crate::py::get(features, "pe_ttm"), 0.0)
        }
    };
    let roe_5y_min = f(crate::py::get(features, "roe_5y_min"), 0.0);
    let roe_5y_avg = f(crate::py::get(features, "roe_5y_avg"), 0.0);
    let mcap_yi = f(crate::py::get(features, "market_cap_yi"), 0.0);
    let rev_g = {
        let a = f(crate::py::get(features, "revenue_growth_3y_cagr"), 0.0);
        if a != 0.0 {
            a
        } else {
            f(crate::py::get(features, "revenue_growth_latest"), 0.0)
        }
    };
    let div_y = f(crate::py::get(features, "dividend_yield"), 0.0);
    let industry = s(crate::py::get(features, "industry"));
    let industry = industry.trim().to_string();
    let market = {
        let m = s(crate::py::get_or(
            features,
            "market",
            &serde_json::json!("A"),
        ));
        if m.is_empty() { "A".to_string() } else { m }
    };

    let hit = |list: &[String]| list.iter().any(|kw| industry.contains(kw.as_str()));

    // 1. Distressed: cheap on book, weak minimum returns (includes negative ROE).
    if pb > 0.0 && pb < 1.0 && roe_5y_min < 5.0 {
        return DISTRESSED.to_string();
    }
    // 2. Quant factor: several quant funds hold it in their top ten.
    if is_quant_factor_style {
        return QUANT_FACTOR.to_string();
    }
    // 3. A-share small-cap speculation.
    if market == "A" && mcap_yi > 0.0 && mcap_yi < 100.0 {
        return SMALL_SPECULATIVE.to_string();
    }
    // 4. Cyclical industry.
    if hit(&style.cycle_industries) {
        return CYCLE.to_string();
    }
    // 5. High-growth tech: 3y CAGR > 20% in a growth industry.
    if rev_g > 20.0 && hit(&style.growth_industries) {
        return GROWTH_TECH.to_string();
    }
    // 6. Dividend defence.
    if div_y > 4.0 && hit(&style.defensive_industries) {
        return DIVIDEND_DEFENSE.to_string();
    }
    // 7. White horse: large, cheap, high returns.
    if mcap_yi > 1000.0 && pe > 0.0 && pe < 25.0 && roe_5y_avg > 12.0 {
        return WHITE_HORSE.to_string();
    }
    BALANCED.to_string()
}

/// Result of the style-weighted re-scoring.
pub struct StyleAdjustment {
    pub panel_consensus: f64,
    pub fundamental_score: f64,
    pub diagnostics: Value,
}

/// `apply_style_weights` — recomputes consensus and fundamental score with
/// school weights times per-investor overrides, and style-specific dimension
/// multipliers.
pub fn apply_style_weights(
    style: &StyleData,
    panel_investors: &[Value],
    dims: &Value,
    style_name: &str,
) -> StyleAdjustment {
    let style_name = if style.group_weights.contains_key(style_name) {
        style_name
    } else {
        BALANCED
    };
    let empty = std::collections::BTreeMap::new();
    let group_w = style.group_weights.get(style_name).unwrap_or(&empty);

    let mut bullish_w = 0.0_f64;
    let mut neutral_w = 0.0_f64;
    let mut active_w = 0.0_f64;
    let mut bullish_n = 0_i64;
    let mut neutral_n = 0_i64;
    let mut bearish_n = 0_i64;

    for inv in panel_investors {
        let sig = s(crate::py::get(inv, "signal"));
        if sig == "skip" {
            continue;
        }
        let gid = s(crate::py::get(inv, "group"));
        let gw = group_w.get(&gid).copied().unwrap_or(1.0);
        // Tuple keys are flattened as "style\u{0}investor" in the data file.
        let key = format!("{style_name}\u{0}{}", s(crate::py::get(inv, "investor_id")));
        let pw = style.person_overrides.get(&key).copied().unwrap_or(1.0);
        let w = gw * pw;
        active_w += w;
        match sig.as_str() {
            "bullish" => {
                bullish_w += w;
                bullish_n += 1;
            }
            "neutral" => {
                // 0.6, aligned with `generate_panel`'s NEUTRAL_WEIGHT.
                neutral_w += w * 0.6;
                neutral_n += 1;
            }
            _ => bearish_n += 1,
        }
    }
    let consensus = if active_w > 0.0 {
        (bullish_w + neutral_w) / active_w.max(0.001) * 100.0
    } else {
        0.0
    };

    let active_n = bullish_n + neutral_n + bearish_n;
    let raw_consensus_old = bullish_n as f64 / active_n.max(1) as f64 * 100.0;

    // Fundamental score re-weighted by style-specific dimension multipliers.
    let dim_mults = style.dim_multipliers.get(style_name);
    let mut tw = 0.0_f64;
    let mut tww = 0.0_f64;
    let mut tw_old = 0.0_f64;
    let mut tww_old = 0.0_f64;
    if let Some(map) = crate::py::get(dims, "dimensions").as_object() {
        for (dim_key, d) in map {
            if !d.is_object() {
                continue;
            }
            let score = f(crate::py::get(d, "score"), 0.0);
            let base_w = {
                let w = f(crate::py::get(d, "weight"), 1.0);
                if crate::py::get(d, "weight").is_null() {
                    1.0
                } else {
                    w
                }
            };
            let mult = dim_mults
                .and_then(|m| m.get(dim_key))
                .copied()
                .unwrap_or(1.0);
            let w = base_w * mult;
            tww += score * w;
            tw += w;
            tww_old += score * base_w;
            tw_old += base_w;
        }
    }
    let fund_score = if tw != 0.0 { tww / tw * 10.0 } else { 0.0 };
    let raw_fund_old = if tw_old != 0.0 {
        tww_old / tw_old * 10.0
    } else {
        0.0
    };

    StyleAdjustment {
        panel_consensus: round_to(consensus, 1),
        fundamental_score: round_to(fund_score, 1),
        diagnostics: serde_json::json!({
            "active_weight": round_to(active_w, 2),
            "bullish_weight": round_to(bullish_w, 2),
            "neutral_weight": round_to(neutral_w, 2),
            "active_count": active_n,
            "bullish_count": bullish_n,
            "neutral_count": neutral_n,
            "bearish_count": bearish_n,
            "raw_consensus_old": round_to(raw_consensus_old, 1),
            "raw_fund_old": round_to(raw_fund_old, 1),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn data() -> crate::panel_data::PanelData {
        crate::panel_data().expect("embedded panel data")
    }

    #[test]
    fn distressed_requires_cheap_book_and_weak_minimum_return() {
        let d = data();
        let s = &d.style;
        assert_eq!(
            detect_style(s, &json!({"pb": 0.8, "roe_5y_min": -2.0}), false),
            DISTRESSED
        );
        // A healthy book value does not trigger it.
        assert_ne!(
            detect_style(s, &json!({"pb": 1.5, "roe_5y_min": -2.0}), false),
            DISTRESSED
        );
    }

    #[test]
    fn a_share_small_cap_is_speculative_before_industry_rules() {
        let d = data();
        let s = &d.style;
        assert_eq!(
            detect_style(
                s,
                &json!({"market": "A", "market_cap_yi": 50.0, "industry": "钢铁"}),
                false
            ),
            SMALL_SPECULATIVE
        );
        // Same size but a US listing falls through.
        assert_ne!(
            detect_style(s, &json!({"market": "US", "market_cap_yi": 50.0}), false),
            SMALL_SPECULATIVE
        );
    }

    #[test]
    fn white_horse_needs_scale_low_pe_and_high_average_roe() {
        let d = data();
        let s = &d.style;
        let big = json!({"market": "A", "market_cap_yi": 5000.0, "pe": 18.0, "roe_5y_avg": 20.0});
        assert_eq!(detect_style(s, &big, false), WHITE_HORSE);
        // Fails the ROE leg.
        let weak = json!({"market": "A", "market_cap_yi": 5000.0, "pe": 18.0, "roe_5y_avg": 8.0});
        assert_eq!(detect_style(s, &weak, false), BALANCED);
    }

    #[test]
    fn quant_factor_short_circuits_the_later_rules() {
        let d = data();
        let s = &d.style;
        // Would otherwise read as a white horse.
        let big = json!({"market": "A", "market_cap_yi": 5000.0, "pe": 18.0, "roe_5y_avg": 20.0});
        assert_eq!(detect_style(s, &big, true), QUANT_FACTOR);
    }

    #[test]
    fn style_weighting_favours_the_matching_school() {
        let d = data();
        let s = &d.style;
        // Quant factor weights group G at 1.5 and the others lower.
        let investors = vec![
            json!({"investor_id": "simons", "group": "G", "signal": "bullish"}),
            json!({"investor_id": "buffett", "group": "A", "signal": "bearish"}),
        ];
        let adj = apply_style_weights(s, &investors, &json!({"dimensions": {}}), QUANT_FACTOR);
        // Simon's school weight (1.5) times his personal override (1.5) gives
        // 2.25 bullish against 2.25 + 0.8 active — 73.8%, confirmed against the
        // Python `apply_style_weights` for the same inputs.
        assert!(
            (adj.panel_consensus - 73.8).abs() < 0.05,
            "{}",
            adj.panel_consensus
        );
    }

    #[test]
    fn unknown_style_falls_back_to_balanced() {
        let d = data();
        let s = &d.style;
        let adj = apply_style_weights(s, &[], &json!({"dimensions": {}}), "not_a_style");
        assert_eq!(adj.panel_consensus, 0.0);
    }
}
