//! Port of `score_fns.generate_panel` (UZI-Skill v3.9.4).
//!
//! Aggregates the 66 per-investor verdicts into the panel view: consensus,
//! vote/signal distributions, per-school scores, the short book, and the
//! hollow-verdict integrity check.
//!
//! Consensus is deliberately a *mixture*, not a plain vote share. The original
//! explains why in a long comment: signal classification discards magnitude
//! (55 and 40 both read as "neutral"), so a vote-only formula bunched every
//! stock into 40–55. The shipped formula keeps both components:
//!
//! ```text
//! consensus = polarize(0.65 * mean(score) + 0.35 * vote_share * 100, k = 1.30)
//! polarize(c, k) = clamp(50 + (c - 50) * k, 0, 100)
//! ```
//!
//! The polarisation stretch is intentional: rule-engine scores cluster near 50,
//! so the distance from 50 is amplified to separate strong from weak.

use serde_json::{Map, Value, json};

use crate::evaluator::{Verdict, evaluate};
use crate::panel_data::PanelData;
use crate::py;
use crate::rules::RuleSet;

const NEUTRAL_WEIGHT: f64 = 0.6;
const SCORE_WEIGHT: f64 = 0.65;
const VOTE_WEIGHT: f64 = 0.35;
const POLARIZE_K: f64 = 1.30;

/// `scenario` → group label/description, used for the school table.
fn group_meta(group: &str) -> (&'static str, &'static str) {
    match group {
        "A" => ("经典价值派", "巴菲特 / 格雷厄姆 / 费雪 / 芒格 一脉"),
        "B" => ("成长派", "彼得·林奇 / 欧奈尔 / 蒂尔 / 伍德 一脉"),
        "C" => ("宏观派", "索罗斯 / 达利欧 / 马克斯 一脉"),
        "D" => ("技术派", "利弗莫尔 / Minervini / 达瓦斯 一脉"),
        "E" => ("中式价投", "段永平 / 张坤 / 朱少醒 / 冯柳 一脉"),
        "F" => ("A 股游资", "龙虎榜顶流 23 位·章盟主/孙哥/赵老哥为代表"),
        "G" => ("量化派", "Simons / Thorp / Shaw 一脉"),
        "H" => ("科技领袖派", "黄仁勋 / 马斯克 / Altman / Saylor 一脉"),
        "I" => ("AI 卡位/瓶颈猎手", "Serenity · AI 供应链卡脖子/瓶颈点"),
        _ => ("", ""),
    }
}

/// `_polarize` — stretch the distance from 50.
fn polarize(c: f64) -> f64 {
    (50.0 + (c - 50.0) * POLARIZE_K).clamp(0.0, 100.0)
}

/// `_score_to_verdict` for a scored, non-skipped investor.
fn score_to_verdict(score: f64, signal: &str) -> &'static str {
    match signal {
        "bullish" if score >= 80.0 => "强烈买入",
        "bullish" => "买入",
        "bearish" if score <= 20.0 => "回避",
        "bearish" => "观望",
        _ => {
            if score >= 50.0 {
                "关注"
            } else {
                "观望"
            }
        }
    }
}

/// `_consensus_to_verdict` for school-level scores (thresholds 80/65/50/35).
fn consensus_to_verdict(c: f64) -> &'static str {
    if c >= 80.0 {
        "重仓"
    } else if c >= 65.0 {
        "买入"
    } else if c >= 50.0 {
        "关注"
    } else if c >= 35.0 {
        "谨慎"
    } else {
        "回避"
    }
}

/// Builds the panel document.
///
/// `comment_for` supplies each investor's persona line. The original picks it
/// with an unseeded `random.choice`, so it is injected here rather than computed:
/// callers choose the policy, which keeps the rest of the document reproducible.
pub fn generate_panel(
    data: &PanelData,
    rules: &RuleSet,
    raw: &Value,
    features: &Value,
    locked_school: Option<&str>,
    mut comment_for: impl FnMut(&str, &str, &Value) -> String,
) -> Value {
    let basic_ctx = py::data(raw, "0_basic");
    let kline_ctx = py::data(raw, "2_kline");
    let fin_ctx = py::data(raw, "1_financials");

    let mut investors_out: Vec<Value> = Vec::new();
    let mut vote_dist = Map::new();
    for k in [
        "strongly_buy",
        "buy",
        "watch",
        "wait",
        "avoid",
        "n_a",
        "skip",
    ] {
        vote_dist.insert(k.to_string(), json!(0));
    }
    let mut sig_dist = Map::new();
    for k in ["bullish", "neutral", "bearish", "skip"] {
        sig_dist.insert(k.to_string(), json!(0));
    }

    // Database order, matching the original's `for inv in INVESTORS`.
    for inv_id in &data.investor_order {
        let meta = &data.investor_meta[inv_id];
        let mandate = meta.mandate.clone();
        let v: Verdict = evaluate(data, rules, inv_id, features, locked_school);

        let (verdict, comment, headline, reasoning);
        if v.signal == "skip" {
            let reason = v.skip_reason.clone().unwrap_or_else(|| "不在能力圈".into());
            verdict = "不适合".to_string();
            headline = format!("不适合 — {reason}");
            comment = format!("不在能力圈范围内，不做评价。\n{headline}");
            reasoning = v.rationale.clone();
        } else {
            verdict = if mandate == "short" && v.signal == "bearish" {
                "做空候选".to_string()
            } else if mandate == "short" {
                "无明确做空逻辑".to_string()
            } else {
                score_to_verdict(v.score, &v.signal).to_string()
            };

            let ctx = json!({
                "name": or_str(py::get(basic_ctx, "name"), "这只票"),
                "industry": or_str(py::get(basic_ctx, "industry"), "该行业"),
                "price": or_str(py::get(basic_ctx, "price"), "—"),
                "pe": or_str(py::get(basic_ctx, "pe_ttm"), "—"),
                "roe": roe_hint(fin_ctx),
                "stage": or_str(py::get(kline_ctx, "stage"), "—"),
                "growth": or_str(py::get(fin_ctx, "revenue_growth"), "—"),
            });
            let persona_line = comment_for(inv_id, &v.signal, &ctx);
            headline = v.headline.clone();
            comment = format!("{persona_line}\n{headline}");
            reasoning = v.rationale.clone();
        }

        let v_key = match verdict.as_str() {
            "强烈买入" => "strongly_buy",
            "买入" => "buy",
            "关注" => "watch",
            "观望" => "wait",
            "回避" => "avoid",
            "不适合" => "skip",
            _ => "n_a",
        };
        if mandate != "short" {
            bump(&mut vote_dist, v_key);
            bump(&mut sig_dist, &v.signal);
        }

        investors_out.push(json!({
            "investor_id": v.investor_id,
            "name": meta.name,
            "group": meta.group,
            "mandate": mandate,
            "avatar": format!("avatars/{inv_id}.svg"),
            "signal": v.signal,
            "confidence": v.confidence,
            "score": if v.signal == "skip" { 0 } else { v.score as i64 },
            "verdict": verdict,
            "reasoning": reasoning,
            "comment": comment,
            "headline": headline,
            "pass": first4(&v.pass_rules),
            "fail": first4(&v.fail_rules),
            "weight_pass": v.weight_pass,
            "weight_total": v.weight_total,
            "ideal_price": Value::Null,
            "period": if matches!(meta.group.as_str(), "A" | "B" | "E") { "中长线" } else { "短线" },
            "time_horizon": v.time_horizon,
            "position_sizing": v.position_sizing,
            "what_would_change_my_mind": v.what_would_change_my_mind,
        }));
    }

    // ── consensus ────────────────────────────────────────────────────
    let bullish = count(&sig_dist, "bullish") as f64;
    let neutral = count(&sig_dist, "neutral") as f64;
    let bearish = count(&sig_dist, "bearish") as f64;
    let active_count = bullish + neutral + bearish;

    let active_scores: Vec<f64> = investors_out
        .iter()
        .filter(|m| {
            py::py_str(py::get(m, "mandate")) != "short"
                && py::py_str(py::get(m, "signal")) != "skip"
        })
        .map(|m| py::f(py::get(m, "score"), 0.0))
        .collect();
    let score_mean = if active_scores.is_empty() {
        50.0
    } else {
        active_scores.iter().sum::<f64>() / active_scores.len() as f64
    };
    let vote_weighted = (bullish + NEUTRAL_WEIGHT * neutral) / active_count.max(1.0) * 100.0;
    let consensus_raw = SCORE_WEIGHT * score_mean + VOTE_WEIGHT * vote_weighted;
    let consensus = polarize(consensus_raw);

    // ── short book ───────────────────────────────────────────────────
    let short_book: Vec<&Value> = investors_out
        .iter()
        .filter(|m| py::py_str(py::get(m, "mandate")) == "short")
        .collect();
    let short_active: Vec<&&Value> = short_book
        .iter()
        .filter(|m| py::py_str(py::get(m, "signal")) != "skip")
        .collect();
    let short_scores: Vec<f64> = short_active
        .iter()
        .map(|m| py::f(py::get(m, "score"), 0.0))
        .collect();
    let mut top_short: Vec<&Value> = short_active.iter().map(|m| **m).collect();
    top_short.sort_by(|a, b| {
        py::f(py::get(a, "score"), 0.0)
            .partial_cmp(&py::f(py::get(b, "score"), 0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let top_short: Vec<Value> = top_short
        .iter()
        .take(5)
        .map(|m| {
            json!({
                "id": py::py_str(py::get(m, "investor_id")),
                "name": py::py_str(py::get(m, "name")),
                "score": py::get(m, "score").clone(),
                "headline": py::py_str(py::get(m, "headline")),
            })
        })
        .collect();
    let short_consensus = json!({
        "total": short_book.len(),
        "active": short_active.len(),
        "skip": short_book.len() - short_active.len(),
        "short_candidates": short_active.iter().filter(|m| py::py_str(py::get(m, "signal")) == "bearish").count(),
        "no_short_thesis": short_active.iter().filter(|m| matches!(py::py_str(py::get(m, "signal")).as_str(), "bullish" | "neutral")).count(),
        "avg_score": if short_scores.is_empty() { 50.0 } else { py::round_to(short_scores.iter().sum::<f64>() / short_scores.len() as f64, 1) },
        "top_short_candidates": top_short,
    });

    // ── per-school scores ────────────────────────────────────────────
    let mut groups: Vec<String> = data
        .investor_meta
        .values()
        .map(|m| m.group.clone())
        .collect();
    groups.sort();
    groups.dedup();

    let mut school_scores = Map::new();
    for g in &groups {
        let members: Vec<&Value> = investors_out
            .iter()
            .filter(|m| {
                py::py_str(py::get(m, "group")) == *g
                    && py::py_str(py::get(m, "mandate")) != "short"
            })
            .collect();
        let all_members: usize = investors_out
            .iter()
            .filter(|m| py::py_str(py::get(m, "group")) == *g)
            .count();
        let active_m: Vec<&&Value> = members
            .iter()
            .filter(|m| py::py_str(py::get(m, "signal")) != "skip")
            .collect();
        let n_active = active_m.len();

        let g_bull = active_m
            .iter()
            .filter(|m| py::py_str(py::get(m, "signal")) == "bullish")
            .count();
        let g_neu = active_m
            .iter()
            .filter(|m| py::py_str(py::get(m, "signal")) == "neutral")
            .count();
        let g_bear = active_m
            .iter()
            .filter(|m| py::py_str(py::get(m, "signal")) == "bearish")
            .count();
        let g_skip = members
            .iter()
            .filter(|m| py::py_str(py::get(m, "signal")) == "skip")
            .count();

        let (s_mean, s_vote, s_consensus) = if n_active > 0 {
            let mean = active_m
                .iter()
                .map(|m| py::f(py::get(m, "score"), 0.0))
                .sum::<f64>()
                / n_active as f64;
            let vote = (g_bull as f64 + NEUTRAL_WEIGHT * g_neu as f64) / n_active as f64 * 100.0;
            (
                mean,
                vote,
                polarize(SCORE_WEIGHT * mean + VOTE_WEIGHT * vote),
            )
        } else {
            (0.0, 0.0, 0.0)
        };

        let dominant = if n_active == 0 {
            "skip"
        } else {
            let mut counts = [("bullish", g_bull), ("neutral", g_neu), ("bearish", g_bear)];
            counts.sort_by_key(|c| std::cmp::Reverse(c.1));
            counts[0].0
        };

        let (label, desc) = group_meta(g);
        school_scores.insert(
            g.clone(),
            json!({
                "group": g,
                "label": label,
                "desc": desc,
                "n_members": members.len(),
                "n_active": n_active,
                "short_excluded": all_members - members.len(),
                "consensus": py::round_to(s_consensus, 1),
                "avg_score": py::round_to(s_mean, 1),
                "vote_consensus": py::round_to(s_vote, 1),
                "score_mean": py::round_to(s_mean, 1),
                "verdict": if n_active > 0 { consensus_to_verdict(s_consensus) } else { "不适合" },
                "bullish": g_bull,
                "neutral": g_neu,
                "bearish": g_bear,
                "skip": g_skip,
                "dominant_signal": dominant,
            }),
        );
    }

    // ── hollow-verdict integrity check ───────────────────────────────
    let active_long: Vec<&Value> = investors_out
        .iter()
        .filter(|m| {
            py::py_str(py::get(m, "mandate")) != "short"
                && py::py_str(py::get(m, "signal")) != "skip"
        })
        .collect();
    let hollow_ids: Vec<String> = active_long
        .iter()
        .filter(|m| {
            py::f(py::get(m, "score"), 0.0) == 0.0
                && py::arr(py::get(m, "pass")).is_empty()
                && py::arr(py::get(m, "fail")).is_empty()
        })
        .map(|m| py::py_str(py::get(m, "investor_id")))
        .collect();
    let hollow_pct = if active_long.is_empty() {
        0.0
    } else {
        py::round_to(
            hollow_ids.len() as f64 / active_long.len() as f64 * 100.0,
            0,
        )
    };
    let consensus_valid = hollow_pct < 20.0;

    let skip_count = count(&sig_dist, "skip");

    json!({
        "ticker": py::get(raw, "ticker").clone(),
        "panel_consensus": py::round_to(consensus, 1),
        "consensus_valid": consensus_valid,
        "hollow_verdicts": hollow_ids.len(),
        "hollow_pct": hollow_pct,
        "hollow_ids": hollow_ids,
        "consensus_warning": if consensus_valid {
            Value::Null
        } else {
            json!(format!(
                "共识分不可采信：{}/{} 位多头评委（{:.0}%）没有任何有效规则证据。",
                hollow_ids.len(),
                active_long.len(),
                hollow_pct
            ))
        },
        "vote_distribution": Value::Object(vote_dist),
        "signal_distribution": Value::Object(sig_dist),
        "investors": investors_out,
        "school_scores": Value::Object(school_scores),
        "long_active": active_count as i64,
        "short_consensus": short_consensus,
        "consensus_formula": {
            "version": "v2.15.5 · polarize(0.65*score_mean + 0.35*vote_weighted, k=1.3)",
            "score_weight": SCORE_WEIGHT,
            "vote_weight": VOTE_WEIGHT,
            "neutral_weight": NEUTRAL_WEIGHT,
            "polarize_k": POLARIZE_K,
            "score_mean": py::round_to(score_mean, 2),
            "vote_weighted": py::round_to(vote_weighted, 2),
            "consensus_raw": py::round_to(consensus_raw, 2),
            "consensus_final": py::round_to(consensus, 2),
            "bullish": bullish as i64,
            "neutral_weighted": py::round_to(neutral * NEUTRAL_WEIGHT, 2),
            "bearish": bearish as i64,
            "skip": skip_count,
            "active": active_count as i64,
            "short_excluded": short_book.len(),
        },
    })
}

fn bump(dist: &mut Map<String, Value>, key: &str) {
    let next = count(dist, key) + 1;
    dist.insert(key.to_string(), json!(next));
}

fn count(dist: &Map<String, Value>, key: &str) -> i64 {
    dist.get(key).and_then(|v| v.as_i64()).unwrap_or(0)
}

fn or_str(v: &Value, default: &str) -> Value {
    if crate::rules::truthy(v) {
        v.clone()
    } else {
        json!(default)
    }
}

/// `str((fin.roe_history or ["—"])[-1])`.
fn roe_hint(fin_ctx: &Value) -> Value {
    let hist = py::arr(py::get(fin_ctx, "roe_history"));
    match hist.last() {
        Some(v) => json!(py::py_str(v)),
        None => json!("—"),
    }
}

fn first4(hits: &[crate::evaluator::RuleHit]) -> Value {
    Value::Array(
        hits.iter()
            .take(4)
            .map(|h| json!({"name": h.name, "msg": h.msg, "weight": h.weight}))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polarize_stretches_away_from_fifty_and_clamps() {
        assert_eq!(polarize(50.0), 50.0);
        // 70 -> 76, 30 -> 24; the stretch is symmetric about 50.
        assert_eq!(polarize(70.0), 76.0);
        assert_eq!(polarize(30.0), 24.0);
        // Clamped at both ends.
        assert_eq!(polarize(100.0), 100.0);
        assert_eq!(polarize(0.0), 0.0);
    }

    #[test]
    fn consensus_mixes_continuous_score_with_vote_share() {
        // All bullish at 90: mean 90, vote share 100.
        let mean = 90.0;
        let vote = 100.0;
        let raw = SCORE_WEIGHT * mean + VOTE_WEIGHT * vote;
        assert_eq!(raw, 93.5);
        assert_eq!(polarize(raw), 100.0);
    }

    #[test]
    fn school_thresholds_match_the_documented_ladder() {
        assert_eq!(consensus_to_verdict(85.0), "重仓");
        assert_eq!(consensus_to_verdict(70.0), "买入");
        assert_eq!(consensus_to_verdict(55.0), "关注");
        assert_eq!(consensus_to_verdict(40.0), "谨慎");
        assert_eq!(consensus_to_verdict(20.0), "回避");
    }

    #[test]
    fn investor_thresholds_cover_every_signal() {
        assert_eq!(score_to_verdict(85.0, "bullish"), "强烈买入");
        assert_eq!(score_to_verdict(70.0, "bullish"), "买入");
        assert_eq!(score_to_verdict(15.0, "bearish"), "回避");
        assert_eq!(score_to_verdict(30.0, "bearish"), "观望");
        assert_eq!(score_to_verdict(60.0, "neutral"), "关注");
        assert_eq!(score_to_verdict(40.0, "neutral"), "观望");
    }
}
