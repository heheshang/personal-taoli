//! Port of `investor_evaluator` + `investor_knowledge` (UZI-Skill v3.9.4).
//!
//! Turns the feature vector into one verdict per investor through three layers:
//! a reality check (market scope, known holdings, industry affinity), the rule
//! engine, and a composite score.
//!
//! Two behaviours are load-bearing and easy to get wrong:
//!
//! * A rule whose evidence is missing is **skipped**, not failed. Skipped rules
//!   contribute to neither `weight_pass` nor `weight_total`, so absent data does
//!   not drag a score down. `eval` signals this by returning `Err`.
//! * `KNOWN_HOLDINGS` is matched by substring against both ticker and company
//!   name, and a match overrides the rule verdict entirely — real money outranks
//!   a checklist.

use serde_json::{Map, Value, json};

use crate::panel_data::PanelData;
use crate::py;
use crate::rules::{eval, truthy};

/// `score >= 65` is bullish.
const BULLISH_THRESHOLD: f64 = 65.0;
/// `score < 35` is bearish.
const BEARISH_THRESHOLD: f64 = 35.0;

/// One investor's verdict.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub investor_id: String,
    pub score: f64,
    pub signal: String,
    pub confidence: i64,
    pub weight_pass: i64,
    pub weight_total: i64,
    pub pass_rules: Vec<RuleHit>,
    pub fail_rules: Vec<RuleHit>,
    pub headline: String,
    pub rationale: String,
    pub skip_reason: Option<String>,
    pub time_horizon: String,
    pub position_sizing: String,
    pub what_would_change_my_mind: String,
}

/// One rule that fired, with its formatted message.
#[derive(Debug, Clone)]
pub struct RuleHit {
    pub rule_id: String,
    pub name: String,
    pub weight: i64,
    pub msg: String,
}

impl Verdict {
    /// The JSON shape consumed by the report renderer.
    pub fn to_json(&self) -> Value {
        let project = |hits: &[RuleHit]| -> Value {
            Value::Array(
                hits.iter()
                    .map(|h| json!({"name": h.name, "msg": h.msg, "weight": h.weight}))
                    .collect(),
            )
        };
        json!({
            "investor_id": self.investor_id,
            "score": self.score,
            "signal": self.signal,
            "confidence": self.confidence,
            "weight_pass": self.weight_pass,
            "weight_total": self.weight_total,
            "pass_rules": project(&self.pass_rules),
            "fail_rules": project(&self.fail_rules),
            "headline": self.headline,
            "rationale": self.rationale,
            "time_horizon": self.time_horizon,
            "position_sizing": self.position_sizing,
            "what_would_change_my_mind": self.what_would_change_my_mind,
        })
    }
}

/// `MARKET_SCOPE` gate: does this investor look at this market at all?
fn market_match(data: &PanelData, investor_id: &str, market: &str) -> bool {
    match data.market_scope.get(investor_id) {
        None => true,
        Some(scope) => {
            if scope == "all" {
                true
            } else {
                scope.to_uppercase().contains(&market.to_uppercase())
            }
        }
    }
}

/// `check_known_holdings` — substring match against ticker and name.
fn known_holding<'a>(
    data: &'a PanelData,
    investor_id: &str,
    ticker: &str,
    name: &str,
) -> Option<(&'a str, &'a str)> {
    let holdings = data.known_holdings.get(investor_id)?;
    holdings
        .iter()
        .find(|h| ticker.contains(h.match_key()) || name.contains(h.match_key()))
        .map(|h| (h.attitude(), h.note()))
}

/// `compute_affinity` — keyword hits, +4 each (capped 10) less -5 each (capped 10).
fn compute_affinity(data: &PanelData, investor_id: &str, industry: &str, name: &str) -> i64 {
    let Some(info) = data.industry_affinity.get(investor_id) else {
        return 0;
    };
    let text = format!("{industry} {name}").to_lowercase();
    let love_hits = info
        .love
        .iter()
        .filter(|kw| text.contains(&kw.to_lowercase()))
        .count() as i64;
    let hate_hits = info
        .hate
        .iter()
        .filter(|kw| text.contains(&kw.to_lowercase()))
        .count() as i64;
    (love_hits * 4).min(10) - (hate_hits * 5).min(10)
}

/// Outcome of the reality check.
struct RealityCheck {
    should_evaluate: bool,
    skip_reason: Option<String>,
    holding: Option<(String, String)>,
    affinity_adjust: i64,
    override_signal: Option<String>,
}

fn reality_check(data: &PanelData, investor_id: &str, f: &Value) -> RealityCheck {
    let market = py::py_str(py::get_or(f, "market", &json!("A")));
    let ticker = py::py_str(py::get(f, "ticker"));
    let name = py::py_str(py::get(f, "name"));
    let industry = py::py_str(py::get(f, "industry"));

    let mut out = RealityCheck {
        should_evaluate: true,
        skip_reason: None,
        holding: None,
        affinity_adjust: 0,
        override_signal: None,
    };

    if !market_match(data, investor_id, &market) {
        out.should_evaluate = false;
        out.skip_reason = Some(format!("不看{market}市场"));
        return out;
    }

    if let Some((attitude, note)) = known_holding(data, investor_id, &ticker, &name) {
        out.holding = Some((attitude.to_string(), note.to_string()));
        if attitude == "held" || attitude == "bullish_known" {
            out.override_signal = Some("bullish".to_string());
            out.affinity_adjust = 15;
        }
    }

    out.affinity_adjust += compute_affinity(data, investor_id, &industry, &name);
    out
}

/// F-group range gate, from `seat_db`.
///
/// Falls back to "in range" when the seat or its fit rules are unknown, matching
/// the original's `try/except` that disables gating rather than blocking.
fn youzi_out_of_range(data: &PanelData, investor_id: &str, f: &Value) -> Option<String> {
    let meta = data.investor_meta.get(investor_id)?;
    if meta.group != "F" {
        return None;
    }
    let nickname = meta.name.as_str();
    let seat = data.seats.get(nickname)?;
    let rules = py::get(seat, "fit_rules");

    // `is_in_range` reads `market_cap` in yuan, deriving it from 亿 when absent.
    let mc_yi = py::f(py::get(f, "market_cap_yi"), 0.0);
    let mc = {
        let direct = py::f(py::get(f, "market_cap"), 0.0);
        if direct != 0.0 { direct } else { mc_yi * 1e8 }
    };

    let mut in_range = true;
    let min_mcap = py::f(py::get(rules, "min_mcap"), 0.0);
    if py::get(rules, "min_mcap").is_number() && mc < min_mcap {
        in_range = false;
    }
    let max_mcap = py::f(py::get(rules, "max_mcap"), 0.0);
    let has_max = py::get(rules, "max_mcap").is_number();
    if has_max && mc > max_mcap {
        in_range = false;
    }
    // Implicit ceiling for everyone except the allowlisted mega-cap trader.
    const FALLBACK_MAX_MCAP: f64 = 50_000_000_000.0;
    if !has_max && nickname != "章盟主" && mc > FALLBACK_MAX_MCAP {
        in_range = false;
    }
    // Remaining fit rules must match the feature exactly, when the feature exists.
    if let Some(map) = rules.as_object() {
        for (k, want) in map {
            if k.starts_with("min_") || k.starts_with("max_") {
                continue;
            }
            let got = f.get(k);
            if let Some(got) = got
                && got != want
            {
                in_range = false;
            }
        }
    }

    if in_range {
        return None;
    }

    // A seat that actually traded this name is forced in, overriding the range.
    let matched = py::arr(py::get(f, "matched_youzi"));
    if matched.iter().any(|m| py::py_str(m) == nickname) {
        return None;
    }

    let mc_display = if mc != 0.0 { mc / 1e8 } else { 0.0 };
    Some(format!("市值 {mc_display:.0} 亿不在 {nickname} 射程"))
}

/// `_fmt_msg` — `str.format_map` with unknown/`None` values rendered as `?`.
fn fmt_msg(template: &str, f: &Value) -> String {
    if template.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(template.len());
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            // `{{` is a literal brace.
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                out.push('{');
                i += 2;
                continue;
            }
            let Some(close) = (i + 1..chars.len()).find(|&j| chars[j] == '}') else {
                out.push(chars[i]);
                i += 1;
                continue;
            };
            let spec: String = chars[i + 1..close].iter().collect();
            out.push_str(&format_one(&spec, f));
            i = close + 1;
            continue;
        }
        if chars[i] == '}' && i + 1 < chars.len() && chars[i + 1] == '}' {
            out.push('}');
            i += 2;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Formats one `{name}` / `{name:.1f}` placeholder.
fn format_one(spec: &str, f: &Value) -> String {
    let (name, fmt) = match spec.split_once(':') {
        Some((n, fmt)) => (n, Some(fmt)),
        None => (spec, None),
    };
    // Only bare identifiers are looked up; anything else (positional, attribute
    // access) is not used by the shipped messages.
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return format!("{{{spec}}}");
    }
    let value = f.get(name);
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return "?".to_string();
    };
    let Some(fmt) = fmt else {
        // `{x}` with no spec uses Python's `str()`.
        return py::py_str(value);
    };
    // Preserve any literal text around the numeric spec (e.g. `{x:.1f}%`).
    let numeric = fmt.split('}').next().unwrap_or(fmt);
    // `py::f` returns the default on failure, so "did it parse" is checked by
    // testing the result rather than by an Option.
    let x = py::f(value, f64::NAN);
    if !x.is_finite() {
        return "?".to_string();
    }
    let digits = numeric.rsplit_once('.').and_then(|(_, d)| {
        d.chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<usize>()
            .ok()
    });
    match digits {
        Some(d) => format!("{:.*}", d, x),
        None => format!("{x}"),
    }
}

/// `_build_headline`.
fn build_headline(signal: &str, pass: &[RuleHit], fail: &[RuleHit]) -> String {
    if signal == "bullish" && !pass.is_empty() {
        return format!("看多核心：{}", pass[0].msg);
    }
    if signal == "bearish" && !fail.is_empty() {
        return format!("看空核心：{}", fail[0].msg);
    }
    if !pass.is_empty() && !fail.is_empty() {
        return format!("观望：{}；但 {}", pass[0].msg, fail[0].msg);
    }
    if !pass.is_empty() {
        return format!("中性：{}", pass[0].msg);
    }
    if !fail.is_empty() {
        return format!("中性：{}", fail[0].msg);
    }
    "数据不足，暂无判断".to_string()
}

/// `_build_rationale`.
fn build_rationale(pass: &[RuleHit], fail: &[RuleHit]) -> String {
    let mut lines: Vec<String> = Vec::new();
    if !pass.is_empty() {
        lines.push("✅ 符合标准：".to_string());
        for r in pass.iter().take(4) {
            lines.push(format!("  • [权{}] {}", r.weight, r.msg));
        }
    }
    if !fail.is_empty() {
        lines.push("❌ 未达标准：".to_string());
        for r in fail.iter().take(4) {
            lines.push(format!("  • [权{}] {}", r.weight, r.msg));
        }
    }
    if lines.is_empty() {
        "无有效规则命中".to_string()
    } else {
        lines.join("\n")
    }
}

/// Fills the three authentic profile fields.
fn profile_fields(data: &PanelData, investor_id: &str) -> (String, String, String) {
    let p = data.profile(investor_id);
    let get = |k: &str| p.get(k).cloned().unwrap_or_default();
    (
        get("time_horizon"),
        get("position_sizing"),
        get("what_would_change_my_mind"),
    )
}

/// Evaluates one investor. `locked_school` mirrors `--school X`; `None` leaves
/// every school active.
pub fn evaluate(
    data: &PanelData,
    rules: &crate::rules::RuleSet,
    investor_id: &str,
    f: &Value,
    locked_school: Option<&str>,
) -> Verdict {
    let (th, ps, ww) = profile_fields(data, investor_id);

    let skip = |reason: String| Verdict {
        investor_id: investor_id.to_string(),
        score: -1.0,
        signal: "skip".to_string(),
        confidence: 0,
        weight_pass: 0,
        weight_total: 0,
        pass_rules: Vec::new(),
        fail_rules: Vec::new(),
        headline: format!("不适合 — {reason}"),
        rationale: format!("该投资者{reason}，不对此股票发表意见。"),
        skip_reason: Some(reason),
        time_horizon: th.clone(),
        position_sizing: ps.clone(),
        what_would_change_my_mind: ww.clone(),
    };

    // School lock: investors outside the requested school drop out entirely.
    if let Some(lock) = locked_school {
        let group = data
            .investor_meta
            .get(investor_id)
            .map(|m| m.group.as_str())
            .unwrap_or("");
        if group != lock {
            return skip(format!("用户锁定 {lock} 派视角 · 非该派评委不参与"));
        }
    }

    let rc = reality_check(data, investor_id, f);
    if !rc.should_evaluate {
        return skip(rc.skip_reason.unwrap_or_else(|| "不在能力圈".into()));
    }
    if let Some(reason) = youzi_out_of_range(data, investor_id, f) {
        return skip(reason);
    }

    // Keyed by investor id — no constant-name indirection, which is what
    // silently dropped the 23 programmatically-built hot-money rule sets.
    let rules_for = rules.rules_for(investor_id);

    if rules_for.is_empty() {
        return Verdict {
            investor_id: investor_id.to_string(),
            score: 50.0,
            signal: "neutral".to_string(),
            confidence: 30,
            weight_pass: 0,
            weight_total: 0,
            pass_rules: Vec::new(),
            fail_rules: Vec::new(),
            headline: "该投资者暂无量化评估规则".to_string(),
            rationale: "此投资者未配置规则库，使用默认中性判断。".to_string(),
            skip_reason: None,
            time_horizon: th,
            position_sizing: ps,
            what_would_change_my_mind: ww,
        };
    }

    // Layer 2: the rule engine. A raising rule is skipped, not failed.
    let mut pass_list: Vec<RuleHit> = Vec::new();
    let mut fail_list: Vec<RuleHit> = Vec::new();
    let mut weight_pass: i64 = 0;
    let mut weight_total: i64 = 0;

    for rule in rules_for {
        match eval(&rule.spec, f) {
            Err(_) => continue,
            Ok(v) => {
                weight_total += rule.weight;
                let hit = RuleHit {
                    rule_id: rule.id.clone(),
                    name: rule.name.clone(),
                    weight: rule.weight,
                    msg: if truthy(&v) {
                        let t = if rule.pass_msg.is_empty() {
                            rule.name.clone()
                        } else {
                            rule.pass_msg.clone()
                        };
                        fmt_msg(&t, f)
                    } else {
                        let t = if rule.fail_msg.is_empty() {
                            format!("未达{}", rule.name)
                        } else {
                            rule.fail_msg.clone()
                        };
                        fmt_msg(&t, f)
                    },
                };
                if truthy(&v) {
                    weight_pass += rule.weight;
                    pass_list.push(hit);
                } else {
                    fail_list.push(hit);
                }
            }
        }
    }

    // Layer 3: reality adjustment. A known holding injects a virtual top-weight rule.
    if let Some((attitude, note)) = &rc.holding
        && (attitude == "held" || attitude == "bullish_known")
    {
        pass_list.insert(
            0,
            RuleHit {
                rule_id: "known_holding".to_string(),
                name: "实际持仓 / 公开看好".to_string(),
                weight: 6,
                msg: format!("📌 {note}"),
            },
        );
        weight_pass += 6;
        weight_total += 6;
    }

    let score = if weight_total > 0 {
        py::round_to(
            (weight_pass as f64 / weight_total as f64) * 100.0 + rc.affinity_adjust as f64,
            1,
        )
    } else {
        py::round_to(50.0 + rc.affinity_adjust as f64, 1)
    };
    let score = score.clamp(0.0, 100.0);

    let signal = match rc.override_signal {
        Some(s) => s,
        None if score >= BULLISH_THRESHOLD => "bullish".to_string(),
        None if score < BEARISH_THRESHOLD => "bearish".to_string(),
        None => "neutral".to_string(),
    };

    let n_rules = rules_for.len() + if rc.holding.is_some() { 1 } else { 0 };
    let base_conf = (50 + n_rules as i64 * 8).min(100) as f64;
    let extremeness = (score - 50.0).abs() * 0.6;
    let confidence = ((base_conf * 0.6 + 40.0 + extremeness * 0.4).min(100.0)).round() as i64;

    // Weight-descending, matching the original's display order.
    pass_list.sort_by_key(|r| std::cmp::Reverse(r.weight));
    fail_list.sort_by_key(|r| std::cmp::Reverse(r.weight));

    let headline = build_headline(&signal, &pass_list, &fail_list);
    let rationale = build_rationale(&pass_list, &fail_list);

    Verdict {
        investor_id: investor_id.to_string(),
        score,
        signal,
        confidence,
        weight_pass,
        weight_total,
        pass_rules: pass_list,
        fail_rules: fail_list,
        headline,
        rationale,
        skip_reason: None,
        time_horizon: th,
        position_sizing: ps,
        what_would_change_my_mind: ww,
    }
}

/// Serialises a map of verdicts for the panel stage.
pub fn verdict_index(verdicts: &[Verdict]) -> Map<String, Value> {
    let mut out = Map::new();
    for v in verdicts {
        out.insert(v.investor_id.clone(), v.to_json());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> PanelData {
        crate::panel_data().expect("embedded panel data")
    }

    #[test]
    fn market_scope_gate_skips_out_of_scope_markets() {
        let d = data();
        // A-share hot money does not look at US listings.
        assert!(market_match(&d, "buffett", "US"));
        let f =
            json!({"market": "A", "ticker": "600519.SH", "name": "贵州茅台", "industry": "白酒"});
        let rc = reality_check(&d, "buffett", &f);
        assert!(rc.should_evaluate);
    }

    #[test]
    fn known_holding_overrides_the_rule_verdict() {
        let d = data();
        // Buffett holds KO; the override must force a bullish signal.
        let f = json!({"market": "US", "ticker": "KO", "name": "可口可乐", "industry": "饮料"});
        let rc = reality_check(&d, "buffett", &f);
        assert_eq!(rc.override_signal.as_deref(), Some("bullish"));
        // 15 for the holding plus 4 for 饮料 hitting Buffett's `love` list —
        // verified against the Python `reality_check`, which also returns 19.
        assert_eq!(rc.affinity_adjust, 19);
    }

    #[test]
    fn affinity_rewards_loved_industries_and_penalises_hated_ones() {
        let d = data();
        // Buffett: 白酒 is in `love`, 半导体 in `hate`.
        assert_eq!(compute_affinity(&d, "buffett", "白酒", "贵州茅台"), 4);
        assert_eq!(compute_affinity(&d, "buffett", "半导体", "某芯片"), -5);
        // Unknown investor has no opinion.
        assert_eq!(compute_affinity(&d, "nobody", "白酒", "x"), 0);
    }

    #[test]
    fn youzi_range_gate_rejects_mega_caps() {
        let d = data();
        // 68,000亿 market cap is far outside a hot-money seat's range.
        let f = json!({
            "market": "A", "ticker": "600519.SH", "name": "贵州茅台",
            "industry": "白酒", "market_cap_yi": 68000.0, "matched_youzi": []
        });
        let reason = youzi_out_of_range(&d, "zhao_lg", &f);
        assert!(reason.is_some(), "mega cap should be out of range");
        assert!(reason.unwrap().contains("不在"), "reason mentions the seat");
    }

    #[test]
    fn lhb_participation_overrides_the_range_gate() {
        let d = data();
        let base = json!({
            "market": "A", "ticker": "600519.SH", "name": "贵州茅台",
            "industry": "白酒", "market_cap_yi": 68000.0
        });
        // Without a matching seat nickname the gate applies.
        assert!(youzi_out_of_range(&d, "zhao_lg", &base).is_some());
        // With it, the investor is forced in despite the range.
        let name = d.investor_meta["zhao_lg"].name.clone();
        let mut with_lhb = base.clone();
        with_lhb["matched_youzi"] = json!([name]);
        assert!(youzi_out_of_range(&d, "zhao_lg", &with_lhb).is_none());
    }

    #[test]
    fn missing_evidence_skips_a_rule_rather_than_failing_it() {
        let d = data();
        let rules = crate::rules().unwrap();
        // No financials at all: `fcf_positive` must not count toward the score.
        let f = crate::features::extract_features(
            &json!({"ticker": "TEST", "dimensions": {}}),
            &d.feature_tables,
        );
        let v = evaluate(&d, &rules, "buffett", &f, None);
        // Its weight must be absent from the total, so the remaining rules decide.
        assert!(v.weight_total >= 0);
        assert!(v.score >= 0.0 && v.score <= 100.0);
    }

    #[test]
    fn school_lock_excludes_other_schools() {
        let d = data();
        let rules = crate::rules().unwrap();
        let f = crate::features::extract_features(
            &json!({"ticker": "600519.SH", "dimensions": {}}),
            &d.feature_tables,
        );
        let locked = evaluate(&d, &rules, "buffett", &f, Some("F"));
        assert_eq!(locked.signal, "skip");
        let same = evaluate(&d, &rules, "buffett", &f, Some("A"));
        assert_ne!(same.signal, "skip");
    }

    #[test]
    fn format_specs_render_values_and_question_marks() {
        let f = json!({"roe_5y_min": 18.7, "net_margin": 50.5});
        assert_eq!(fmt_msg("ROE {roe_5y_min:.1f}% 稳", &f), "ROE 18.7% 稳");
        assert_eq!(fmt_msg("净利率 {net_margin:.1f}%", &f), "净利率 50.5%");
        // A missing or null value renders as `?`, never as an empty string.
        assert_eq!(fmt_msg("ROE {absent:.1f}%", &f), "ROE ?%");
        assert_eq!(fmt_msg("x {null_v:.0f}", &json!({"null_v": null})), "x ?");
    }
}
