//! Port of `score_fns.generate_synthesis` + `compute_friendly` (UZI-Skill v3.9.4).
//!
//! Merges the scored dimensions and panel into the document the report renderer
//! consumes: verdict, debate, risks, buy zones, friendly scenarios, dashboard.
//!
//! Two seams are parameters rather than computations, because in the original
//! they reach outside the analysis:
//!
//! * `is_quant_factor_style` — `detect_quant_signal` fetches fund top-10
//!   holdings over the network.
//! * `persona_comment` — `get_comment` picks a line with an *unseeded*
//!   `random.choice`, so its output is not reproducible by construction.
//!
//! Agent overrides (`agent_analysis.json`) are accepted and applied where the
//! original applies them; passing `Value::Null` means "script-only", which is
//! the path the reference artifacts were produced on.

use serde_json::{Map, Value, json};

use crate::panel_data::PanelData;
use crate::py::{self, get, get_or, py_str, py_str_or_empty, round_to};
use crate::style::{StyleAdjustment, apply_style_weights, detect_style};

/// Inputs the analysis layer cannot compute itself.
pub struct SynthesisInputs<'a> {
    /// From `detect_quant_signal` (network).
    pub is_quant_factor_style: bool,
    /// Persona line for (investor_id, signal, ctx).
    pub persona_comment: &'a dyn Fn(&str, &str, &Value) -> String,
    /// `agent_analysis.json`, or `Null` for script-only synthesis.
    pub agent_analysis: &'a Value,
    /// `_data_gaps.json`, merged by the original's orchestrator (`run_real_test`)
    /// rather than by `generate_synthesis`. `Null` when absent.
    pub data_gaps: &'a Value,
}

/// `run_real_test`'s `_data_gaps.json` merge, including agent acknowledgements.
fn merge_data_gaps(gaps_doc: &Value, agent_analysis: &Value) -> Option<Value> {
    if !gaps_doc.is_object() {
        return None;
    }
    let acks = get(agent_analysis, "data_gap_acknowledged");
    let tasks: Vec<Value> = crate::py::arr(get(gaps_doc, "tasks"))
        .iter()
        .map(|t| {
            let mut t = t.clone();
            let key = format!("{}.{}", py_str(get(&t, "dim")), py_str(get(&t, "field")));
            let dim_key = py_str(get(&t, "dim"));
            let acked =
                crate::rules::truthy(get(acks, &key)) || crate::rules::truthy(get(acks, &dim_key));
            if acked {
                t["status"] = json!("acknowledged");
                let note = if crate::rules::truthy(get(acks, &key)) {
                    get(acks, &key)
                } else {
                    get(acks, &dim_key)
                };
                t["agent_note"] = note.clone();
            }
            t
        })
        .collect();
    let unresolved = tasks
        .iter()
        .filter(|t| py_str(get(t, "status")) == "pending")
        .count();
    Some(json!({
        "coverage_pct": get(gaps_doc, "coverage_pct").clone(),
        "total_gaps": tasks.len(),
        "unresolved": unresolved,
        "tasks": tasks,
    }))
}

/// `_parse_pct` — strips `%` and `+`.
fn parse_pct(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s
            .chars()
            .filter(|c| *c != '%' && *c != '+')
            .collect::<String>()
            .trim()
            .parse()
            .unwrap_or(0.0),
        _ => 0.0,
    }
}

fn data_of<'a>(raw: &'a Value, dim: &str) -> &'a Value {
    let d = get(get(raw, "dimensions"), dim);
    let payload = get(d, "data");
    if payload.is_object() {
        payload
    } else {
        &Value::Null
    }
}

/// `compute_scenarios` — five bands derived from annualised volatility.
fn compute_scenarios(raw: &Value) -> Value {
    let basic = data_of(raw, "0_basic");
    let kline = data_of(raw, "2_kline");
    let research = data_of(raw, "6_research");

    let entry_price = get(basic, "price").clone();
    let stats = get(kline, "kline_stats");
    let sigma = {
        let v = parse_pct(get_or(stats, "volatility", &json!("30%")));
        if v != 0.0 { v } else { 30.0 }
    };
    let base_return = {
        let v = parse_pct(get_or(research, "upside", &json!("+15%")));
        if v != 0.0 { v } else { 15.0 }
    };

    json!({
        "entry_price": entry_price,
        "cases": [
            {"name": "最坏情况", "probability": "5%",  "return": round_to(-2.0 * sigma, 1)},
            {"name": "偏差情况", "probability": "25%", "return": round_to(-sigma + base_return * 0.2, 1)},
            {"name": "合理情况", "probability": "40%", "return": round_to(base_return, 1)},
            {"name": "乐观情况", "probability": "25%", "return": round_to(1.0 * sigma + base_return * 0.5, 1)},
            {"name": "极致乐观", "probability": "5%",  "return": round_to(2.0 * sigma + base_return, 1)},
        ],
    })
}

/// `compute_exit_triggers` — five conditions straight from the data.
fn compute_exit_triggers(raw: &Value, features: &Value) -> Vec<String> {
    let basic = data_of(raw, "0_basic");
    let kline = data_of(raw, "2_kline");
    let val = data_of(raw, "10_valuation");
    let lhb = data_of(raw, "16_lhb");
    let research = data_of(raw, "6_research");
    let market = py_str(get_or(raw, "market", &json!("A")));
    let cur = match market.as_str() {
        "H" => "HK$",
        "U" => "$",
        _ => "¥",
    };

    let price = py::f(get(basic, "price"), 0.0);
    let mut triggers: Vec<String> = Vec::new();

    // 1. Technical stop. A 60-day MA above the current price means the line is
    //    already broken, so the stop belongs below the price instead.
    let ma60 = crate::py::arr(get(kline, "ma60_60d"));
    let ma60_last = ma60
        .iter()
        .rev()
        .find(|v| crate::rules::truthy(v))
        .map(|v| py::f(v, 0.0));
    match ma60_last {
        Some(ma) if price > 0.0 && ma < price => {
            triggers.push(format!(
                "股价跌破 {cur}{ma:.2}（60 日均线支撑位）→ 无条件止损"
            ));
        }
        _ if price > 0.0 => {
            triggers.push(format!(
                "股价跌破 {cur}{:.2}（当前价 -12%）→ 无条件止损",
                price * 0.88
            ));
        }
        _ => triggers.push("股价放量跌破 60 日均线 → 无条件止损".to_string()),
    }

    // 2. Deteriorating fundamentals, read from revenue growth.
    let rev_g = py::f(get(features, "revenue_growth_latest"), 0.0);
    if rev_g < 0.0 {
        triggers.push(format!(
            "营收同比已转负（-{:.1}%）→ 基本面反转信号",
            rev_g.abs()
        ));
    } else {
        triggers.push("下季度营收同比转负 → 基本面反转信号".to_string());
    }

    // 3. Earnings guidance failure.
    let g = parse_pct(get_or(research, "upside", &json!("+15%")));
    if g > 0.0 {
        let min_growth = 10.max((g - 15.0) as i64);
        triggers.push(format!("下次业绩预告低于 +{min_growth}% → 预期管理失守"));
    } else {
        triggers.push("连续两期业绩不及券商预期中位数 → 逻辑失效".to_string());
    }

    // 4. Hot-money exit.
    let matched = get(lhb, "matched_youzi");
    let matched_str = if let Some(list) = matched.as_array() {
        list.iter()
            .take(2)
            .map(py_str)
            .collect::<Vec<_>>()
            .join(" / ")
    } else {
        // `str(matched).split("/")[0] if matched else "顶级游资"` — an absent or
        // empty value must fall through to the default, not render as "None".
        let raw_str = py_str_or_empty(matched);
        if raw_str.is_empty() {
            "顶级游资".to_string()
        } else {
            raw_str.split('/').next().unwrap_or("").to_string()
        }
    };
    if !matched_str.is_empty() && matched_str != "—" {
        triggers.push(format!(
            "{matched_str} 席位大额卖出 > 2 亿 → 顶级资金撤离信号"
        ));
    } else if lhb.is_object() {
        triggers.push("龙虎榜游资席位出现大额净卖出 → 资金撤离信号".to_string());
    }

    // 5. Valuation bubble, keyed on the 5-year PE percentile.
    let pe_quantile = py_str(get(val, "pe_quantile"));
    match first_percentile(&pe_quantile) {
        Some(q) if q >= 80 => {
            triggers.push(format!("PE 已处于 5 年 {q} 分位 → 泡沫区获利了结"));
        }
        Some(q) => {
            triggers.push(format!(
                "PE 站上 5 年 {} 分位 → 泡沫区获利了结",
                (q + 15).min(90)
            ));
        }
        None => triggers.push("PE 站上 5 年 90 分位 → 泡沫区获利了结".to_string()),
    }

    triggers.truncate(5);
    triggers
}

/// `re.search(r"(\d+)\s*分位", s)`.
fn first_percentile(s: &str) -> Option<i64> {
    let chars: Vec<char> = s.chars().collect();
    let marker: Vec<char> = "分位".chars().collect();
    let mut run = String::new();
    for (i, c) in chars.iter().enumerate() {
        if c.is_ascii_digit() {
            run.push(*c);
            continue;
        }
        if !run.is_empty() {
            // Digits must be separated from 分位 by whitespace only.
            let rest: String = chars[i..]
                .iter()
                .collect::<String>()
                .trim_start()
                .to_string();
            if rest.starts_with(&marker.iter().collect::<String>()) {
                return run.parse().ok();
            }
            run.clear();
        }
    }
    None
}

/// `_auto_summarize_dim` — one paragraph per dimension from raw fields.
fn auto_summarize_dim(dim_key: &str, label: &str, dim: &Value, score: f64) -> String {
    if !dim.is_object() {
        return String::new();
    }
    let data = get(dim, "data");
    if !data.is_object() || data.as_object().map(|m| m.is_empty()).unwrap_or(true) {
        return format!("{label}：未拉取到数据（fetcher 失败或返回空）。");
    }

    // `_v(*keys, default="—")` — first key with a non-placeholder value.
    let v = |keys: &[&str]| -> Value {
        for k in keys {
            let val = get(data, k);
            let placeholder = match val {
                Value::Null => true,
                Value::String(s) => matches!(s.as_str(), "" | "—" | "-"),
                Value::Array(a) => a.is_empty(),
                Value::Object(o) => o.is_empty(),
                _ => false,
            };
            if !placeholder {
                return val.clone();
            }
        }
        json!("—")
    };
    let v_default = |keys: &[&str], default: &str| -> Value {
        let got = v(keys);
        if got == json!("—") {
            json!(default)
        } else {
            got
        }
    };
    let join_list = |items: Vec<String>, sep: &str| -> Option<String> {
        if items.is_empty() {
            return None;
        }
        Some(items.into_iter().take(5).collect::<Vec<_>>().join(sep))
    };
    let s = |val: &Value| -> String {
        match val {
            Value::String(x) => x.clone(),
            other => py_str(other),
        }
    };

    match dim_key {
        "0_basic" => format!(
            "{label}：{}（{}），{} 行业。市值 {}，PE {}，PB {}。",
            s(&v(&["name"])),
            s(&v(&["code"])),
            s(&v(&["industry"])),
            s(&v(&["market_cap"])),
            s(&v(&["pe_ttm"])),
            s(&v(&["pb"]))
        ),
        "1_financials" => format!(
            "{label}：ROE {}，营收同比 {}，净利同比 {}，净利率 {}。综合得分 {score}/10。",
            s(&v(&["roe_latest", "roe"])),
            s(&v(&["revenue_growth_yoy", "revenue_yoy"])),
            s(&v(&["net_profit_yoy"])),
            s(&v(&["net_margin", "gross_margin"]))
        ),
        "2_kline" => format!(
            "{label}：{} · 均线 {} · MACD {}。",
            s(&v(&["stage", "wyckoff_stage"])),
            s(&v(&["ma_align", "trend"])),
            s(&v(&["macd"]))
        ),
        "3_macro" => format!(
            "{label}：利率周期 {}；汇率 {}；地缘 {}；大宗商品 {}。得分 {score}/10。",
            s(&v(&["rate_cycle"])),
            s(&v(&["fx_trend"])),
            s(&v(&["geo_risk"])),
            s(&v(&["commodity", "commodity_trend"]))
        ),
        "4_peers" => {
            let peers: Vec<String> = crate::py::arr(get(data, "peer_table"))
                .iter()
                .filter(|p| p.is_object() && !crate::rules::truthy(get(p, "is_self")))
                .filter_map(|p| get(p, "name").as_str().map(str::to_string))
                .take(5)
                .collect();
            let peers_str = join_list(peers, "、");
            format!(
                "{label}：{} 行业，{}{}。得分 {score}/10。",
                s(&v(&["industry"])),
                s(&v(&["rank"])),
                peers_str
                    .map(|p| format!("，主要同行：{p}"))
                    .unwrap_or_default()
            )
        }
        "5_chain" => format!(
            "{label}：上游 {}；下游 {}；客户集中度 {}。",
            s(&v(&["upstream"])),
            s(&v(&["downstream"])),
            s(&v(&["client_concentration"]))
        ),
        "6_research" => format!(
            "{label}：近期券商研报 {} 篇，一致评级 {}，目标价均值 {}。",
            s(&v(&["report_count", "n_reports"])),
            s(&v(&["consensus_rating", "rating"])),
            s(&v(&["avg_target_price", "target_price"]))
        ),
        "7_industry" => {
            let cninfo = get(data, "cninfo_metrics");
            // The original writes `_v(...) or (data.get("cninfo_metrics") or {})...`,
            // but `_v`'s default is "—" (truthy), so the fallback is unreachable.
            // Mirrored as-is: adding the fallback changes output to "None".
            let _ = cninfo;
            let ind_pe = v(&["industry_pe_weighted"]);
            let ind_count = v(&["total_companies"]);
            format!(
                "{label}：所属 {} · 行业 PE 加权 {} · 上市公司数 {} · 增速 {}。",
                s(&v(&["industry"])),
                s(&ind_pe),
                s(&ind_count),
                s(&v(&["growth"]))
            )
        }
        "8_materials" => format!(
            "{label}：核心原料 {}；近期价格走势 {}；占成本比例 {}。",
            s(&v(&["core_material"])),
            s(&v(&["price_trend"])),
            s(&v(&["cost_share"]))
        ),
        "9_futures" => format!(
            "{label}：关联合约 {}；近期走势 {}；{}。",
            s(&v(&["linked_contract"])),
            s(&v(&["contract_trend"])),
            s(&v_default(&["note"], ""))
        ),
        "10_valuation" => format!(
            "{label}：PE 5 年分位 {}，PB 5 年分位 {}。得分 {score}/10。",
            s(&v(&["pe_quantile_5y", "pe_quantile"])),
            s(&v(&["pb_quantile_5y", "pb_quantile"]))
        ),
        "11_governance" => format!(
            "{label}：实控人 {}；近期变动 {}。",
            s(&v(&["actual_controller"])),
            s(&v(&["recent_changes", "recent_holdings_change"]))
        ),
        "12_capital_flow" => {
            let north = v(&["north_holding_pct", "north_change_5d"]);
            let margin = v(&["margin_balance"]);
            if north != json!("—") || margin != json!("—") {
                format!(
                    "{label}：北向持股 {}；融资余额 {}。",
                    if north == json!("—") {
                        "—".to_string()
                    } else {
                        s(&north)
                    },
                    if margin == json!("—") {
                        "—".to_string()
                    } else {
                        s(&margin)
                    }
                )
            } else {
                format!("{label}：{}。", s(&v_default(&["_note"], "资金面数据有限")))
            }
        }
        "13_policy" => {
            let snippets = get(data, "snippets");
            let non_empty: Vec<(String, usize)> = snippets
                .as_object()
                .map(|m| {
                    m.iter()
                        .filter(|(_, val)| crate::rules::truthy(val))
                        .map(|(k, val)| {
                            let n = match val {
                                Value::Array(a) => a.len(),
                                _ => 1,
                            };
                            (k.clone(), n)
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !non_empty.is_empty() {
                let preview = non_empty
                    .iter()
                    .map(|(k, n)| format!("{k}: {n} 条"))
                    .collect::<Vec<_>>()
                    .join("；");
                format!(
                    "{label}：{} {} 年政策检索：{preview}。",
                    s(&v_default(&["industry"], "本行业")),
                    s(&v_default(&["year"], ""))
                )
            } else {
                format!(
                    "{label}：{} 政策搜索未命中具体内容（建议 web_search 补抓）。",
                    s(&v_default(&["industry"], "本行业"))
                )
            }
        }
        "14_moat" => {
            let scores = get(data, "scores");
            let total: Option<f64> = scores
                .as_object()
                .map(|m| m.values().map(|x| py::f(x, 0.0)).sum());
            match total {
                Some(t) => format!(
                    "{label}：四力评分 无形资产 {}/10、转换成本 {}/10、网络效应 {}/10、规模 {}/10 · 综合 {t}/40。",
                    s(get(scores, "intangible")),
                    s(get(scores, "switching")),
                    s(get(scores, "network")),
                    s(get(scores, "scale"))
                ),
                None => format!("{label}：评估数据有限，得分 {score}/10。"),
            }
        }
        "15_events" => {
            let timeline = crate::py::arr(get(data, "event_timeline"));
            let recent_news = crate::py::arr(get(data, "recent_news"));
            if !timeline.is_empty() {
                let head = timeline
                    .iter()
                    .take(3)
                    .map(|t| py_str(t).chars().take(60).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("；");
                format!("{label}：近期事件 {} 条，含：{head}。", timeline.len())
            } else if !recent_news.is_empty() {
                let head = recent_news
                    .iter()
                    .take(3)
                    .map(|n| py_str(get(n, "title")).chars().take(60).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("；");
                format!("{label}：近期新闻 {} 条，含：{head}。", recent_news.len())
            } else {
                format!("{label}：暂无显著事件（fetcher 返回空）。")
            }
        }
        "16_lhb" => {
            let n = v(&["recent_lhb_count", "n_lhb_30d"]);
            let seats = {
                let a = get(data, "recent_seats");
                if a.is_array() {
                    a
                } else {
                    get(data, "top_seats")
                }
            };
            if n != json!("—") || !crate::py::arr(seats).is_empty() {
                let seat_str = crate::py::arr(seats)
                    .iter()
                    .take(3)
                    .filter(|s| s.is_object())
                    .map(|s| py_str(get(s, "name")))
                    .collect::<Vec<_>>()
                    .join("、");
                format!(
                    "{label}：近 30 天上榜 {} 次{}。",
                    if n == json!("—") {
                        "—".to_string()
                    } else {
                        s(&n)
                    },
                    if seat_str.is_empty() {
                        String::new()
                    } else {
                        format!("，主要席位：{seat_str}")
                    }
                )
            } else {
                format!("{label}：近期未上龙虎榜或非 A 股。")
            }
        }
        "17_sentiment" => format!(
            "{label}：热度 {}；情绪 {}。",
            s(&v(&["hot_rank", "hot_score"])),
            s(&v(&["sentiment_label", "sentiment"]))
        ),
        "18_trap" => {
            let level = s(&v(&["trap_level", "level"]));
            let scanned = {
                let s_hit = get(data, "signals_hit");
                if s_hit.is_null() {
                    json!("?/8")
                } else {
                    s_hit.clone()
                }
            };
            let rec = s(&v(&["recommendation"]));
            let detail = crate::py::arr(get(data, "signals_hit_detail"));
            let scanned_s = s(&scanned);
            if !detail.is_empty() {
                let kws = detail
                    .iter()
                    .take(3)
                    .map(|d| py_str(get(d, "name")))
                    .collect::<Vec<_>>()
                    .join("、");
                format!("{label}：{level} · 8 信号扫描命中 {scanned_s}（{kws}）· 建议：{rec}")
            } else {
                format!(
                    "{label}：{level} · 8 信号扫描命中 {scanned_s}（已扫 ddgs 24 条搜索结果）· 建议：{rec}"
                )
            }
        }
        "19_contests" => {
            let summary = get(data, "summary");
            let n_cubes = py::f(get(summary, "xueqiu_cubes_total"), 0.0);
            let n_high = py::f(get(summary, "high_return_cubes"), 0.0);
            let login_req = crate::rules::truthy(get(summary, "xueqiu_login_required"));
            let src = py_str(get_or(summary, "xueqiu_source", &json!("http")));
            if login_req && n_cubes == 0.0 {
                format!(
                    "{label}：⚠️ XueQiu cubes 接口需登录（2026 起新政），未启用 → 0 cube。\
                     启用方式：export UZI_XQ_LOGIN=1 + python -m lib.xueqiu_browser login"
                )
            } else if n_cubes != 0.0 {
                format!(
                    "{label}：雪球 {n_cubes:.0} 个组合持有本股（高收益 >50% 的有 {n_high:.0} 个）· 来源 {src}"
                )
            } else {
                format!("{label}：雪球 0 个组合持有本股（可能小盘 / 冷门 / 接口未返）")
            }
        }
        _ => {
            let mut items = Vec::new();
            if let Some(map) = data.as_object() {
                for (k, val) in map.iter().take(5) {
                    let placeholder = match val {
                        Value::Null => true,
                        Value::String(s) => matches!(s.as_str(), "" | "—" | "-"),
                        Value::Array(a) => a.is_empty(),
                        Value::Object(o) => o.is_empty(),
                        _ => false,
                    };
                    if !placeholder && !k.starts_with('_') {
                        let text = py_str(val);
                        items.push(format!("{k}={}", text.chars().take(30).collect::<String>()));
                    }
                }
            }
            if items.is_empty() {
                String::new()
            } else {
                format!("{label}：{}。", items.join("、"))
            }
        }
    }
}

/// Dimension key → display label, in the original's order.
const DIM_LABELS: &[(&str, &str)] = &[
    ("0_basic", "基础信息"),
    ("1_financials", "财报"),
    ("2_kline", "K线技术面"),
    ("3_macro", "宏观环境"),
    ("4_peers", "同行对比"),
    ("5_chain", "产业链"),
    ("6_research", "券商研报"),
    ("7_industry", "行业景气"),
    ("8_materials", "原材料"),
    ("9_futures", "期货关联"),
    ("10_valuation", "估值分位"),
    ("11_governance", "治理/减持"),
    ("12_capital_flow", "资金面"),
    ("13_policy", "政策与监管"),
    ("14_moat", "护城河"),
    ("15_events", "事件驱动"),
    ("16_lhb", "龙虎榜"),
    ("17_sentiment", "舆情"),
    ("18_trap", "杀猪盘"),
    ("19_contests", "实盘比赛"),
];

/// Builds the synthesis document.
#[allow(clippy::too_many_arguments)]
pub fn generate_synthesis(
    data: &PanelData,
    raw: &Value,
    dims_scored: &Value,
    panel: &Value,
    features: &Value,
    inputs: &SynthesisInputs<'_>,
) -> Value {
    let ag = inputs.agent_analysis;

    // Agent per-investor overrides win over the rule engine.
    let mut panel = panel.clone();
    let per_investor = get(ag, "per_investor_override");
    if let Some(overrides) = per_investor.as_object()
        && !overrides.is_empty()
        && let Some(investors) = get(&panel, "investors").as_array().cloned()
    {
        let mut updated = investors;
        for (id, ov) in overrides {
            let Some(inv) = updated
                .iter_mut()
                .find(|i| py_str(get(i, "investor_id")) == *id)
            else {
                continue;
            };
            for fld in [
                "signal",
                "score",
                "headline",
                "reasoning",
                "comment",
                "verdict",
            ] {
                let val = get(ov, fld);
                if !val.is_null() {
                    inv[fld] = val.clone();
                }
            }
            if !crate::rules::truthy(get(inv, "comment"))
                && crate::rules::truthy(get(inv, "headline"))
            {
                let h = get(inv, "headline").clone();
                inv["comment"] = h;
            }
        }
        panel["investors"] = Value::Array(updated);
    }

    let basic = data_of(raw, "0_basic");
    let name = {
        let n = get(basic, "name");
        if crate::rules::truthy(n) {
            py_str(n)
        } else {
            py_str(get(raw, "ticker"))
        }
    };
    let price = py::f(get(basic, "price"), 0.0);

    // ── style detection and re-weighting ──────────────────────────────
    let _fund_score_initial = py::f(get(dims_scored, "fundamental_score"), 60.0);
    let _consensus_initial = py::f(get(&panel, "panel_consensus"), 50.0);

    let mcap_yi = if crate::rules::truthy(get(basic, "market_cap_raw")) {
        py::f(get(basic, "market_cap_raw"), 0.0) / 1e8
    } else {
        0.0
    };
    let d_fin = get(get(dims_scored, "dimensions"), "1_financials");
    let style_features = json!({
        "code": py_str(get(raw, "ticker")),
        "market": py_str(get_or(raw, "market", &json!("A"))),
        "industry": py_str(get(basic, "industry")),
        "market_cap_yi": mcap_yi,
        "pe": py::f(get(basic, "pe_ttm"), 0.0),
        "pe_ttm": py::f(get(basic, "pe_ttm"), 0.0),
        "pb": py::f(get(basic, "pb"), 0.0),
        "roe_5y_avg": py::f(get(d_fin, "roe_5y_avg"), 0.0),
        "roe_5y_min": py::f(get(d_fin, "roe_5y_min"), 0.0),
        "revenue_growth_3y_cagr": py::f(get(d_fin, "revenue_growth_3y_cagr"), 0.0),
        "dividend_yield": py::f(get(basic, "dividend_yield_ttm"), 0.0),
    });

    let style_label = detect_style(&data.style, &style_features, inputs.is_quant_factor_style);
    let investors_for_style: Vec<Value> = get(&panel, "investors")
        .as_array()
        .cloned()
        .unwrap_or_default();
    let StyleAdjustment {
        panel_consensus: consensus,
        fundamental_score: fund_score,
        diagnostics: style_diag,
    } = apply_style_weights(&data.style, &investors_for_style, dims_scored, &style_label);

    let overall = fund_score * 0.6 + consensus * 0.4;

    let mut verdict_label = if overall >= 80.0 {
        "值得重仓"
    } else if overall >= 70.0 {
        "可以蹲一蹲"
    } else if overall >= 65.0 {
        "可以蹲（偏弱）"
    } else if overall >= 60.0 {
        "观望偏多"
    } else if overall >= 55.0 {
        "观望中性"
    } else if overall >= 50.0 {
        "观望偏空"
    } else if overall >= 35.0 {
        "谨慎"
    } else {
        "回避"
    }
    .to_string();

    // School-divergence suffix.
    let school_scores = get(&panel, "school_scores");
    if let Some(map) = school_scores.as_object()
        && !map.is_empty()
    {
        let bullish_schools: Vec<String> = map
            .values()
            .filter(|s| matches!(py_str(get(s, "verdict")).as_str(), "重仓" | "买入"))
            .map(|s| py_str(get(s, "label")))
            .collect();
        let bearish_schools: Vec<String> = map
            .values()
            .filter(|s| py_str(get(s, "verdict")) == "回避")
            .map(|s| py_str(get(s, "label")))
            .collect();
        if !bullish_schools.is_empty() && !bearish_schools.is_empty() {
            verdict_label.push_str(&format!(
                " · {} 派看多 / {} 派看空",
                bullish_schools.len(),
                bearish_schools.len()
            ));
        } else if !bullish_schools.is_empty() {
            verdict_label.push_str(&format!(" · {} 派看多", bullish_schools.len()));
        } else if !bearish_schools.is_empty() {
            verdict_label.push_str(&format!(" · {} 派看空", bearish_schools.len()));
        }
    }
    let verdict_detail = format!("基本面 {fund_score:.1} · 共识 {consensus:.1}");

    // ── bull / bear selection ────────────────────────────────────────
    let investors: Vec<Value> = get(&panel, "investors")
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut eligible: Vec<Value> = investors
        .iter()
        .filter(|i| py_str(get(i, "signal")) != "skip" && py::f(get(i, "score"), 0.0) > 0.0)
        .cloned()
        .collect();
    if eligible.is_empty() {
        eligible = investors
            .iter()
            .filter(|i| py_str(get(i, "signal")) != "skip")
            .cloned()
            .collect();
        if eligible.is_empty() {
            eligible = investors.clone();
        }
    }
    eligible.sort_by(|a, b| {
        py::f(get(b, "score"), 0.0)
            .partial_cmp(&py::f(get(a, "score"), 0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut bull = eligible.first().cloned().unwrap_or_else(|| json!({}));
    let mut bear = eligible.last().cloned().unwrap_or_else(|| json!({}));
    if py_str(get(&bull, "investor_id")) == py_str(get(&bear, "investor_id")) && eligible.len() > 1
    {
        bear = eligible[eligible.len() - 2].clone();
    }
    let _ = &mut bull;

    let gd_override = get(ag, "great_divide_override");
    let agent_bull: Vec<Value> = crate::py::arr(get(gd_override, "bull_say_rounds")).to_vec();
    let agent_bear: Vec<Value> = crate::py::arr(get(gd_override, "bear_say_rounds")).to_vec();

    let join_msgs = |rules: &Value| -> String {
        crate::py::arr(rules)
            .iter()
            .take(3)
            .map(|r| {
                let msg = get(r, "msg");
                if crate::rules::truthy(msg) {
                    py_str(msg)
                } else {
                    py_str(get(r, "name"))
                }
            })
            .collect::<Vec<_>>()
            .join(" · ")
    };
    let round_say = |agent: &[Value], i: usize, fallback: String| -> Value {
        match agent.get(i) {
            Some(v) => v.clone(),
            None => json!(fallback),
        }
    };

    let bull_headline = {
        let h = get(&bull, "headline");
        if crate::rules::truthy(h) {
            py_str(h)
        } else {
            py_str(get(&bull, "comment"))
        }
    };
    let bear_headline = {
        let h = get(&bear, "headline");
        if crate::rules::truthy(h) {
            py_str(h)
        } else {
            py_str(get(&bear, "comment"))
        }
    };

    let bull_pass_joined = join_msgs(get(&bull, "pass"));
    let bear_fail_joined = join_msgs(get(&bear, "fail"));
    let rounds = json!([
        {
            "round": 1,
            "bull_say": round_say(&agent_bull, 0, bull_headline.clone()),
            "bear_say": round_say(&agent_bear, 0, bear_headline.clone()),
        },
        {
            "round": 2,
            "bull_say": round_say(&agent_bull, 1,
                if bull_pass_joined.is_empty() { "数据支持我的判断。".to_string() } else { bull_pass_joined }),
            "bear_say": round_say(&agent_bear, 1,
                if bear_fail_joined.is_empty() { "风险点太多。".to_string() } else { bear_fail_joined }),
        },
        {
            "round": 3,
            "bull_say": round_say(&agent_bull, 2,
                format!("综合看，{} 分，我的立场不变。", py::f(get(&bull, "score"), 0.0))),
            "bear_say": round_say(&agent_bear, 2,
                format!("综合看，{} 分，风险大于收益。", py::f(get(&bear, "score"), 0.0))),
        },
    ]);

    let kline = data_of(raw, "2_kline");
    let d20 = data_of(raw, "20_valuation_models");
    let d21 = data_of(raw, "21_research_workflow");
    let d22 = data_of(raw, "22_deep_methods");
    let dcf_summary = get(d20, "summary");
    let init_cov = get(d21, "initiating_coverage");
    let ic_memo = get(d22, "ic_memo");
    let competitive = get(d22, "competitive_analysis");

    let dcf_sm = py::f(get(dcf_summary, "dcf_safety_margin_pct"), 0.0);
    let lbo_irr = py::f(get(dcf_summary, "lbo_irr_pct"), 0.0);
    let init_headline = get(init_cov, "headline");
    let tp = py::f(get(init_headline, "target_price"), 0.0);
    let upside = py::f(get(init_headline, "upside_pct"), 0.0);
    let rating = py_str(get(init_headline, "rating"));

    let agent_punchline = py_str_or_empty(get(gd_override, "punchline"));
    let punchline = if !agent_punchline.is_empty() {
        agent_punchline
    } else if dcf_sm != 0.0 && lbo_irr != 0.0 && dcf_sm.abs() > 10.0 && lbo_irr > 15.0 {
        if dcf_sm < 0.0 && lbo_irr > 20.0 {
            format!(
                "DCF 说高估 {:.0}%，但 LBO 测试显示 PE 买方仍能赚 {lbo_irr:.0}% IRR — 冲突很有意思。",
                dcf_sm.abs()
            )
        } else if dcf_sm > 15.0 && lbo_irr > 20.0 {
            format!("DCF 认为低估 {dcf_sm:.0}%，LBO IRR {lbo_irr:.0}% 也确认 — 双重信号看多。")
        } else {
            format!(
                "机构建模定调 {rating}，目标价 ¥{tp}（{upside:+.0}%），LBO 视角 IRR {lbo_irr:.0}%。"
            )
        }
    } else if tp > 0.0 && upside.abs() > 5.0 {
        format!("首次覆盖 {rating}，目标价 ¥{tp}，空间 {upside:+.0}%。")
    } else {
        format!("{name} · ROE 历史与当前估值存在结构性分歧，等待方向明朗。")
    };

    // ── risks ────────────────────────────────────────────────────────
    let narrative_override = get(ag, "narrative_override");
    let mut risks: Vec<String> = crate::py::arr(get(narrative_override, "risks"))
        .iter()
        .map(py_str)
        .collect();
    if risks.is_empty()
        && let Some(dims) = get(dims_scored, "dimensions").as_object()
    {
        let mut keys: Vec<&String> = dims.keys().collect();
        keys.sort();
        for key in keys {
            let dim = &dims[key];
            if py::f(get(dim, "score"), 0.0) <= 4.0 {
                let reasons = crate::py::arr(get(dim, "reasons_fail"));
                if let Some(first) = reasons.first() {
                    risks.push(py_str(first));
                } else {
                    let dim_name = {
                        let n = get(dim, "name");
                        if crate::rules::truthy(n) {
                            py_str(n)
                        } else {
                            py_str(get(dim, "label"))
                        }
                    };
                    risks.push(format!(
                        "{dim_name} 评分偏低 ({}/10)",
                        py::f(get(dim, "score"), 0.0)
                    ));
                }
            }
        }
    }
    if risks.is_empty() {
        let pe_val = py::f(get(features, "pe"), 0.0);
        let debt_val = py::f(get(features, "debt_ratio"), 0.0);
        let roe_min = py::f(get(features, "roe_5y_min"), 0.0);
        let industry = {
            let i = py_str(get(features, "industry"));
            if i.is_empty() {
                "所属行业".to_string()
            } else {
                i
            }
        };
        if pe_val > 30.0 {
            risks.push(format!("当前 PE {pe_val:.0}x，估值偏高"));
        }
        if debt_val > 50.0 {
            risks.push(format!("资产负债率 {debt_val:.0}%，财务杠杆偏高"));
        }
        if roe_min < 5.0 {
            risks.push(format!("ROE 最低 {roe_min:.1}%，盈利稳定性不足"));
        }
        risks.push(format!("{industry}行业竞争加剧风险"));
        risks.push("宏观经济或政策环境变化".to_string());
    }
    risks.truncate(5);

    // ── friendly layer ───────────────────────────────────────────────
    let scenarios = compute_scenarios(raw);
    let exit_triggers = compute_exit_triggers(raw, features);
    let similar_stocks = get(raw, "similar_stocks").clone();

    let ytd_return = py_str(get(get(kline, "kline_stats"), "ytd_return"));
    let agent_core = py_str_or_empty(get(narrative_override, "core_conclusion"));
    let long_active = {
        let v = py::f(get(&panel, "long_active"), 0.0);
        if v != 0.0 {
            v as i64
        } else {
            let dist = get(&panel, "signal_distribution");
            ["bullish", "neutral", "bearish"]
                .iter()
                .map(|k| py::f(get(dist, k), 0.0) as i64)
                .sum()
        }
    };
    let core_conclusion = if !agent_core.is_empty() {
        agent_core
    } else {
        format!(
            "{name} · {} 分 · {verdict_label}。{long_active} 位多头评委里 {} 人看多，YTD {ytd_return}。{punchline}",
            overall as i64,
            py::f(get(get(&panel, "signal_distribution"), "bullish"), 0.0) as i64
        )
    };

    // ── per-dimension commentary ─────────────────────────────────────
    let agent_dim = get(ag, "dim_commentary");
    let mut dim_commentary = Map::new();
    for (dim_key, label) in DIM_LABELS {
        // `if dim_key in agent_dim_commentary and agent_dim_commentary[dim_key]`
        // — membership plus truthiness, not "the rendered string is non-empty"
        // (`str(None)` is "None", which is non-empty).
        let provided = get(agent_dim, dim_key);
        if crate::rules::truthy(provided) {
            dim_commentary.insert(dim_key.to_string(), provided.clone());
            continue;
        }
        // `raw["dimensions"].get(k) or {}` — a missing dimension becomes an
        // empty object, which the summariser then reports as "no data pulled".
        // Leaving it Null would drop the key from the report entirely.
        let dim = {
            let d = get(get(raw, "dimensions"), dim_key);
            if d.is_object() { d.clone() } else { json!({}) }
        };
        let score = py::f(
            get(get(get(dims_scored, "dimensions"), dim_key), "score"),
            0.0,
        );
        let auto = auto_summarize_dim(dim_key, label, &dim, score);
        if !auto.is_empty() {
            dim_commentary.insert(dim_key.to_string(), json!(auto));
        }
    }

    let agent_reviewed = crate::rules::truthy(get(ag, "agent_reviewed"));
    let data_gaps = merge_data_gaps(inputs.data_gaps, ag);
    let style_label_cn = data
        .style
        .labels
        .get(&style_label)
        .cloned()
        .unwrap_or_else(|| "?".to_string());
    let style_explanation = data
        .style
        .explanations
        .get(&style_label)
        .cloned()
        .unwrap_or_default();

    // Hoisted: `json!` cannot contain statements.
    let catalysts = {
        let events = crate::py::arr(get(get(d21, "catalyst_calendar"), "events"));
        let list: Vec<String> = events
            .iter()
            .filter(|e| matches!(py_str(get(e, "impact")).as_str(), "high" | "medium"))
            .map(|e| {
                py_str(get_or(e, "event", &json!("季报")))
                    .chars()
                    .take(30)
                    .collect::<String>()
            })
            .take(3)
            .collect();
        if list.is_empty() {
            json!(["暂无明确催化剂"])
        } else {
            json!(list)
        }
    };

    let mut doc = json!({
        "ticker": py_str(get(raw, "ticker")),
        "name": name,
        "overall_score": round_to(overall, 1),
        "verdict_label": verdict_label,
        "verdict_detail": verdict_detail,
        "fundamental_score": round_to(fund_score, 1),
        "panel_consensus": round_to(consensus, 1),
        "school_lock": Value::Null,
        "school_scores": school_scores.clone(),
        "short_consensus": get(&panel, "short_consensus").clone(),
        "dim_commentary": Value::Object(dim_commentary),
        "institutional_modeling": {
            "dcf_intrinsic": get(dcf_summary, "dcf_intrinsic").clone(),
            "dcf_safety_margin_pct": get(dcf_summary, "dcf_safety_margin_pct").clone(),
            "dcf_verdict": get(dcf_summary, "dcf_verdict").clone(),
            "lbo_irr_pct": get(dcf_summary, "lbo_irr_pct").clone(),
            "lbo_verdict": get(dcf_summary, "lbo_verdict").clone(),
            "comps_verdict": get(dcf_summary, "comps_verdict").clone(),
            "initiating_rating": get(init_headline, "rating").clone(),
            "target_price": get(init_headline, "target_price").clone(),
            "upside_pct": get(init_headline, "upside_pct").clone(),
            "ic_recommendation": get(get(get(ic_memo, "sections"), "I_exec_summary"), "headline").clone(),
            "bcg_position": get(get(competitive, "bcg_position"), "category").clone(),
            "industry_attractiveness": get(competitive, "industry_attractiveness_pct").clone(),
        },
        "detected_style": style_label,
        "style_label_cn": style_label_cn,
        "style_explanation": style_explanation,
        "style_diagnostics": style_diag,
        "agent_reviewed": agent_reviewed,
        "panel_insights": py_str_or_empty(get(ag, "panel_insights")),
        "claude_narrative_stub": {
            "_note": if agent_reviewed {
                "以下字段已由 agent 覆盖"
            } else {
                "以下字段是脚本生成的占位，Task 4 中 Claude 必须根据原始数据重写"
            },
            "needs_rewrite": if agent_reviewed {
                json!([])
            } else {
                json!([
                    "great_divide.punchline", "dashboard.core_conclusion",
                    "debate.rounds[*].bull_say", "debate.rounds[*].bear_say",
                    "buy_zones.*.rationale", "risks[*]"
                ])
            },
        },
        "debate": {
            "bull": {
                "investor_id": get(&bull, "investor_id").clone(),
                "name": get(&bull, "name").clone(),
                "group": get(&bull, "group").clone(),
            },
            "bear": {
                "investor_id": get(&bear, "investor_id").clone(),
                "name": get(&bear, "name").clone(),
                "group": get(&bear, "group").clone(),
            },
            "rounds": rounds,
            "punchline": punchline,
        },
        "great_divide": {
            "bull_avatar": get(&bull, "investor_id").clone(),
            "bear_avatar": get(&bear, "investor_id").clone(),
            "bull_score": get(&bull, "score").clone(),
            "bear_score": get(&bear, "score").clone(),
            "bull_signal": get(&bull, "signal").clone(),
            "bear_signal": get(&bear, "signal").clone(),
            "punchline": punchline,
        },
        "risks": risks,
        "buy_zones": if crate::rules::truthy(get(narrative_override, "buy_zones")) {
            get(narrative_override, "buy_zones").clone()
        } else {
            json!({
                "value": {"price": if price > 0.0 { json!(round_to(price * 0.85, 2)) } else { json!("—") }, "rationale": "历史 PE 25 分位"},
                "growth": {"price": if price > 0.0 { json!(round_to(price * 0.92, 2)) } else { json!("—") }, "rationale": "PEG 合理区"},
                "technical": {"price": if price > 0.0 { json!(round_to(price * 0.95, 2)) } else { json!("—") }, "rationale": "MA60 支撑位"},
                "youzi": {"price": if price > 0.0 { json!(price) } else { json!("—") }, "rationale": "当前情绪未破"},
            })
        },
        "friendly": {
            "scenarios": scenarios,
            "exit_triggers": exit_triggers,
            "similar_stocks": similar_stocks,
        },
        "fund_managers": get(raw, "fund_managers").clone(),
        "dashboard": {
            "core_conclusion": core_conclusion,
            "data_perspective": {
                "trend": py_str(get_or(kline, "stage", &json!("—"))),
                "price": if price > 0.0 { format!("¥{price}") } else { "—".to_string() },
                "volume": "—",
                "chips": py_str(get_or(kline, "ma_align", &json!("—"))),
            },
            "intelligence": {
                "news": format!(
                    "已采集 {} 项催化剂事件",
                    crate::py::arr(get(get(d21, "catalyst_calendar"), "events")).len()
                ),
                "risks": risks.iter().take(3).cloned().collect::<Vec<_>>(),
                "catalysts": catalysts,
            },
            "battle_plan": {
                "entry": if price > 0.0 { format!("¥{}", round_to(price * 0.92, 2)) } else { "¥—".to_string() },
                "position": "分批建仓 · 勿满仓",
                "stop": if price > 0.0 { format!("¥{}", round_to(price * 0.85, 2)) } else { "¥—".to_string() },
                "target": if price > 0.0 { format!("¥{}", round_to(price * 1.25, 2)) } else { "¥—".to_string() },
            },
        },
    });

    // Merged last, and only when the collection stage recorded gaps — matching
    // the orchestrator, which omits the key entirely otherwise.
    if let Some(gaps) = data_gaps {
        doc["data_gaps"] = gaps;
    }
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenarios_are_derived_from_volatility_and_upside() {
        let raw = json!({"dimensions": {
            "0_basic": {"data": {"price": 100}},
            "2_kline": {"data": {"kline_stats": {"volatility": "30%"}}},
            "6_research": {"data": {"upside": "+15%"}},
        }});
        let s = compute_scenarios(&raw);
        assert_eq!(s["entry_price"], 100);
        let cases = s["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 5);
        assert_eq!(cases[0]["return"], -60.0);
        assert_eq!(cases[2]["return"], 15.0);
        assert_eq!(cases[4]["return"], 75.0);
        // Probabilities sum to 100%.
        assert_eq!(cases[0]["probability"], "5%");
        assert_eq!(cases[2]["probability"], "40%");
    }

    #[test]
    fn exit_trigger_stop_sits_below_price_when_ma_is_broken() {
        // A 60-day MA above the price means the line is already broken, so the
        // stop must be derived from the price instead.
        let raw = json!({"market": "A", "dimensions": {
            "0_basic": {"data": {"price": 100}},
            "2_kline": {"data": {"ma60_60d": [120.0]}},
            "6_research": {"data": {"upside": "+15%"}},
            "10_valuation": {"data": {"pe_quantile": "50 分位"}},
        }});
        let f = json!({"revenue_growth_latest": 5.0});
        let triggers = compute_exit_triggers(&raw, &f);
        assert!(triggers[0].contains("88.00"), "{}", triggers[0]);
        assert!(!triggers[0].contains("120.00"));
    }

    #[test]
    fn negative_revenue_growth_reports_the_real_figure() {
        let raw = json!({"market": "A", "dimensions": {"0_basic": {"data": {"price": 10}}}});
        let f = json!({"revenue_growth_latest": -7.5});
        let triggers = compute_exit_triggers(&raw, &f);
        assert!(triggers[1].contains("7.5"), "{}", triggers[1]);
    }

    #[test]
    fn percentile_parser_needs_the_unit_suffix() {
        assert_eq!(first_percentile("50 分位"), Some(50));
        assert_eq!(first_percentile("PE 80分位"), Some(80));
        // A bare number is not a percentile.
        assert_eq!(first_percentile("50"), None);
        assert_eq!(first_percentile(""), None);
    }

    #[test]
    fn auto_summary_reports_missing_data_explicitly() {
        let dim = json!({"data": {}});
        let text = auto_summarize_dim("1_financials", "财报", &dim, 5.0);
        assert!(text.contains("未拉取到数据"), "{text}");
    }

    #[test]
    fn auto_summary_places_holders_for_each_dimension() {
        let dim = json!({"data": {"roe_latest": "32.5%", "revenue_growth_yoy": 1.5}});
        let text = auto_summarize_dim("1_financials", "财报", &dim, 8.0);
        assert!(text.starts_with("财报：ROE 32.5%"), "{text}");
        assert!(text.contains("8/10"), "{text}");
    }
}
