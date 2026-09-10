//! Port of `score_fns.score_dimensions` (UZI-Skill v3.9.4).
//!
//! Faithful to the original, including behaviours that look like defects:
//! `"多头" in ma_align` also matches `"非多头"`, several dimensions are
//! hardcoded stubs that return a fixed score whether or not their fetcher
//! produced data, and `_f` silently turns unparseable input into `0.0`.
//! Reproducing those is the point — the reference artifacts were produced by
//! that code, and "fixing" them here would make the port unverifiable.

use serde_json::{Map, Value, json};

use crate::py;

/// Dimensions the original emits without `reasons_pass` / `reasons_fail` keys.
/// Keeping the exact key set matters: the reference JSON omits them.
fn scored(score: i64, weight: i64, label: String) -> Value {
    json!({ "score": score, "weight": weight, "label": label })
}

fn scored_with_reasons(
    score: i64,
    weight: i64,
    label: String,
    pass: Vec<String>,
    fail: Vec<String>,
) -> Value {
    json!({
        "score": score,
        "weight": weight,
        "label": label,
        "reasons_pass": pass,
        "reasons_fail": fail,
    })
}

/// Python's `str()` for a summed numeric, which drops the `.0` of whole floats.
fn fmt_sum(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        py::py_str(&json!(x))
    }
}

/// `score_dimensions(raw) -> {"ticker", "fundamental_score", "dimensions"}`.
pub fn score_dimensions(raw: &Value) -> anyhow::Result<Value> {
    let ticker = raw
        .get("ticker")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("raw data is missing `ticker`"))?;

    let mut out = Map::new();

    // ── 1 · 财报 ──────────────────────────────────────────────
    let fin = py::data(raw, "1_financials");
    let roe = py::f(py::get(fin, "roe"), 0.0);
    let roe_history = py::get(fin, "roe_history");
    let last_roe = match py::arr(roe_history).last() {
        Some(v) => py::f(v, 0.0),
        None => roe,
    };
    let net_margin = py::f(py::get(fin, "net_margin"), 0.0);
    let debt = py::f(py::get(py::get(fin, "financial_health"), "debt_ratio"), 0.0);
    let rev_hist = py::arr(py::get(fin, "revenue_history"));
    // `growth` stays 0 unless there are two periods and the earlier one is truthy.
    let growth = if rev_hist.len() >= 2 {
        let last = py::f(&rev_hist[rev_hist.len() - 1], 0.0);
        let prev = py::f(&rev_hist[rev_hist.len() - 2], 0.0);
        if prev != 0.0 {
            (last - prev) / prev * 100.0
        } else {
            0.0
        }
    } else {
        0.0
    };

    let mut score_1: i64 = 5;
    if last_roe >= 15.0 {
        score_1 += 2;
    } else if last_roe >= 10.0 {
        score_1 += 1;
    } else if last_roe < 5.0 {
        score_1 -= 2;
    }
    if net_margin >= 15.0 {
        score_1 += 1;
    }
    if growth >= 20.0 {
        score_1 += 1;
    }
    if debt >= 60.0 {
        score_1 -= 1;
    }
    let score_1 = score_1.clamp(1, 10);

    let mut pass_1 = Vec::new();
    let mut fail_1 = Vec::new();
    if last_roe >= 15.0 {
        pass_1.push(format!("ROE 最新 {last_roe:.1}%"));
    } else if last_roe < 8.0 {
        fail_1.push(format!("ROE 最新 {last_roe:.1}% 偏低"));
    }
    if growth >= 20.0 {
        pass_1.push(format!("营收增速 {growth:.1}%"));
    } else if growth < 5.0 {
        fail_1.push(format!("营收增速 {growth:.1}% 停滞"));
    }
    if debt < 40.0 {
        pass_1.push(format!("资产负债率 {debt:.0}% 健康"));
    } else if debt > 60.0 {
        fail_1.push(format!("资产负债率 {debt:.0}% 偏高"));
    }
    out.insert(
        "1_financials".into(),
        scored_with_reasons(
            score_1,
            5,
            format!("ROE {last_roe:.1}% · 营收增速 {growth:+.1}% · 负债率 {debt:.0}%"),
            pass_1,
            fail_1,
        ),
    );

    // ── 2 · K 线 ──────────────────────────────────────────────
    let kline = py::data(raw, "2_kline");
    let stage = py::py_str(py::get_or(kline, "stage", &Value::String(String::new())));
    let ma_align = py::py_str(py::get_or(kline, "ma_align", &Value::String(String::new())));
    let stats = py::get(kline, "kline_stats");
    let mut score_2: i64 = 5;
    if stage.contains("Stage 2") {
        score_2 += 2;
    } else if stage.contains("Stage 1") {
        score_2 += 1;
    } else if stage.contains("Stage 3") || stage.contains("Stage 4") {
        score_2 -= 2;
    }
    if ma_align.contains("多头") {
        score_2 += 1;
    }
    let dd = py::f(py::get_or(stats, "max_drawdown", &json!("0%")), 0.0);
    if dd <= -30.0 {
        score_2 -= 1;
    }
    let score_2 = score_2.clamp(1, 10);
    let mut label_2 = format!("{stage} · 均线{ma_align}");
    let ytd = py::get(stats, "ytd_return");
    if !ytd.is_null() && !is_falsy(ytd) {
        label_2.push_str(&format!(" · YTD {}", py::py_str(ytd)));
    }
    out.insert(
        "2_kline".into(),
        scored_with_reasons(
            score_2,
            4,
            label_2,
            if stage.contains("Stage 2") {
                vec![stage.clone()]
            } else {
                Vec::new()
            },
            if dd <= -25.0 {
                vec![format!("最大回撤 {dd:.1}%")]
            } else {
                Vec::new()
            },
        ),
    );

    // ── 3 · 宏观（定性固定值）──────────────────────────────────
    out.insert("3_macro".into(), scored(6, 3, "宏观环境中性".into()));

    // ── 4 · 同行 ──────────────────────────────────────────────
    let peers = py::data(raw, "4_peers");
    let peer_table = py::arr(py::get(peers, "peer_table"));
    let global_peer_count = py::int_or_zero(py::get(
        py::get(peers, "global_peer_comparison"),
        "peer_count",
    ));
    let mut score_4: i64 = 5;
    if !peer_table.is_empty() && peer_table.len() > 1 {
        score_4 = 7;
        if let Some(self_row) = peer_table.iter().find(|p| !is_falsy(py::get(p, "is_self"))) {
            let self_pe = py::f(py::get(self_row, "pe"), 0.0);
            let others: Vec<&Value> = peer_table
                .iter()
                .filter(|p| is_falsy(py::get(p, "is_self")))
                .collect();
            let avg_pe: f64 = others
                .iter()
                .map(|p| py::f(py::get(p, "pe"), 0.0))
                .sum::<f64>()
                / others.len().max(1) as f64;
            if self_pe > 0.0 && avg_pe > 0.0 {
                if self_pe < avg_pe * 0.9 {
                    score_4 += 1;
                } else if self_pe > avg_pe * 1.2 {
                    score_4 -= 1;
                }
            }
        }
    } else if global_peer_count >= 3 {
        score_4 = 7;
    }
    let local_peer_count = (peer_table.len() as i64 - 1).max(0);
    let peer_label = if global_peer_count != 0 {
        format!("全球同行 {global_peer_count} 家对比")
    } else if local_peer_count != 0 {
        format!("同业 {local_peer_count} 家对比")
    } else {
        "无同行数据".to_string()
    };
    out.insert(
        "4_peers".into(),
        scored_with_reasons(score_4, 4, peer_label, Vec::new(), Vec::new()),
    );

    // ── 5 · 上下游 ────────────────────────────────────────────
    let chain = py::data(raw, "5_chain");
    let breakdown = py::arr(py::get(chain, "main_business_breakdown"));
    let score_5 = if !breakdown.is_empty() { 6 } else { 5 };
    out.insert(
        "5_chain".into(),
        scored_with_reasons(
            score_5,
            4,
            if !breakdown.is_empty() {
                format!("主营 {} 类业务已识别", breakdown.len())
            } else {
                "产业链数据不完整".to_string()
            },
            Vec::new(),
            Vec::new(),
        ),
    );

    // ── 6 · 研报 ──────────────────────────────────────────────
    let research = py::data(raw, "6_research");
    let zero = json!(0);
    let coverage_raw = py::get_or(research, "report_count", &zero);
    let coverage = py::int(coverage_raw, 0);
    let ratings = py::get(research, "rating_distribution");
    let mut buy_count = 0.0_f64;
    if let Some(map) = py::obj(ratings) {
        for (k, v) in map {
            if k.contains("买入") || k.contains("增持") {
                buy_count += py::f(v, 0.0);
            }
        }
    }
    let mut score_6: i64 = 5 + (coverage / 5).min(3);
    if buy_count >= 10.0 {
        score_6 += 1;
    }
    let score_6 = score_6.min(10);
    let coverage_truthy = !is_falsy(coverage_raw);
    out.insert(
        "6_research".into(),
        scored_with_reasons(
            score_6,
            3,
            if coverage_truthy {
                format!(
                    "{} 份研报 · 买入/增持 {} 份",
                    py::py_str(coverage_raw),
                    fmt_sum(buy_count)
                )
            } else {
                "研报数据稀少".to_string()
            },
            if coverage >= 10 {
                vec![format!("覆盖券商 {} 家", py::py_str(coverage_raw))]
            } else {
                Vec::new()
            },
            if coverage_truthy {
                Vec::new()
            } else {
                vec!["缺乏覆盖".to_string()]
            },
        ),
    );

    // ── 7/8/9 · 定性固定值 ────────────────────────────────────
    out.insert("7_industry".into(), scored(7, 4, "行业处于成长期".into()));
    out.insert(
        "8_materials".into(),
        scored(6, 3, "原材料成本关注中".into()),
    );
    out.insert("9_futures".into(), scored(5, 2, "无强关联期货品种".into()));

    // ── 10 · 估值 ─────────────────────────────────────────────
    let val = py::data(raw, "10_valuation");
    let pe_q_str = py::py_str(py::get_or(
        val,
        "pe_quantile",
        &Value::String(String::new()),
    ));
    let pe_q = py::first_digits(&pe_q_str).unwrap_or(50);
    let score_10 = if pe_q < 30 {
        9
    } else if pe_q < 50 {
        7
    } else if pe_q < 70 {
        5
    } else if pe_q < 85 {
        3
    } else {
        2
    };
    out.insert(
        "10_valuation".into(),
        scored_with_reasons(
            score_10,
            5,
            format!(
                "PE {} · 5 年 {pe_q} 分位 · 行业均值 {}",
                py::py_str(py::get_or(val, "pe", &Value::String("—".into()))),
                py::py_str(py::get_or(val, "industry_pe", &Value::String("—".into())))
            ),
            if pe_q < 50 {
                vec!["PE 在 5 年中位数以下".to_string()]
            } else {
                Vec::new()
            },
            if pe_q >= 75 {
                vec!["PE 已在 5 年高位区".to_string()]
            } else {
                Vec::new()
            },
        ),
    );

    // ── 11 · 治理 ─────────────────────────────────────────────
    let gov = py::data(raw, "11_governance");
    let pledge = py::get(gov, "pledge");
    let pledge_list = py::arr(pledge);
    let has_insider = !is_falsy(py::get(gov, "insider_trades_1y"));
    let mut score_11: i64 = 6;
    // `not pledge` is Python truthiness: an absent/empty list or a falsey scalar.
    if is_falsy(pledge) || pledge_list.is_empty() {
        score_11 += 1;
    }
    if has_insider {
        score_11 += 1;
    }
    let pledge_display = if pledge.is_array() {
        format!("{}", pledge_list.len())
    } else {
        "—".to_string()
    };
    out.insert(
        "11_governance".into(),
        scored(
            score_11.min(10),
            4,
            format!(
                "质押记录 {} · 内部交易 {}",
                pledge_display,
                if has_insider { "有" } else { "无" }
            ),
        ),
    );

    // ── 12 · 资金面 ───────────────────────────────────────────
    let cap = py::data(raw, "12_capital_flow");
    let main_flow = py::arr(py::get(cap, "main_fund_flow_20d"));
    let mut main_5d_net = 0.0_f64;
    for rec in main_flow.iter().take(5) {
        let v = if rec.is_object() {
            py::get(rec, "主力净流入-净额")
        } else {
            &Value::Null
        };
        match v {
            Value::Number(n) => main_5d_net += n.as_f64().unwrap_or(0.0),
            Value::String(s) => {
                if let Ok(parsed) = s.trim().parse::<f64>() {
                    main_5d_net += parsed;
                }
            }
            _ => {}
        }
    }
    let main_5d_label = if main_5d_net != 0.0 {
        format!("{:+.1}亿", main_5d_net / 1e8)
    } else {
        "—".to_string()
    };
    let unlock = py::arr(py::get(cap, "unlock_schedule"));
    let mut score_12: i64 = 5;
    if main_5d_net > 0.0 {
        score_12 += 2;
    } else if main_5d_net < 0.0 {
        score_12 -= 1;
    }
    if unlock.is_empty() {
        score_12 += 1;
    }
    let score_12 = score_12.clamp(1, 10);
    out.insert(
        "12_capital_flow".into(),
        scored_with_reasons(
            score_12,
            4,
            format!("主力 5日 {main_5d_label} · 12 个月解禁 {} 次", unlock.len()),
            if main_5d_net > 0.0 {
                vec![format!("主力资金 5 日净流入 {main_5d_label}")]
            } else {
                Vec::new()
            },
            if main_5d_net < 0.0 {
                vec![format!("主力资金 5 日净流出 {main_5d_label}")]
            } else {
                Vec::new()
            },
        ),
    );

    // ── 13/14 · 定性固定值 ────────────────────────────────────
    out.insert("13_policy".into(), scored(6, 3, "政策环境中性".into()));
    out.insert("14_moat".into(), scored(6, 3, "护城河需定性评估".into()));

    // ── 15 · 事件 ─────────────────────────────────────────────
    let events = py::data(raw, "15_events");
    let news = py::arr(py::get(events, "news"));
    let notices = py::arr(py::get(events, "recent_notices"));
    let score_15 = 5 + ((news.len() as i64) / 10).min(3);
    out.insert(
        "15_events".into(),
        scored(
            score_15,
            4,
            format!("近期新闻 {} 条 · 公告 {} 份", news.len(), notices.len()),
        ),
    );

    // ── 16 · 龙虎榜 ───────────────────────────────────────────
    let lhb = py::data(raw, "16_lhb");
    let lhb_count = py::int(py::get_or(lhb, "lhb_count_30d", &json!(0)), 0);
    let matched: Vec<String> = py::arr(py::get(lhb, "matched_youzi"))
        .iter()
        .map(py::py_str)
        .collect();
    let mut score_16: i64 = 5 + (lhb_count / 2).min(3);
    if !matched.is_empty() {
        score_16 += 1;
    }
    out.insert(
        "16_lhb".into(),
        json!({
            "score": score_16.min(10),
            "weight": 4,
            "label": format!("近 30 天上榜 {lhb_count} 次 · 识别游资 {} 位", matched.len()),
            "reasons_pass": if matched.is_empty() {
                Vec::<String>::new()
            } else {
                vec![format!("{} 席位出现", matched.iter().take(3).cloned().collect::<Vec<_>>().join("/"))]
            },
        }),
    );

    // ── 17 · 舆情 ─────────────────────────────────────────────
    let hot = py::data(raw, "17_sentiment");
    let hot_rank = py::arr(py::get(py::get(hot, "hot_rank"), "rank_history"));
    let score_17 = 6 + ((hot_rank.len() as i64) / 10).min(2);
    out.insert(
        "17_sentiment".into(),
        scored(score_17, 3, format!("雪球热度上榜 {} 次", hot_rank.len())),
    );

    // ── 18 · 杀猪盘（默认安全）──────────────────────────────────
    out.insert("18_trap".into(), scored(9, 5, "🟢 未发现推广痕迹".into()));

    // ── 19 · 实盘赛 ───────────────────────────────────────────
    let contests = py::data(raw, "19_contests");
    let summary = py::get(contests, "summary");
    let xq_total = py::int(py::get_or(summary, "xueqiu_cubes_total", &json!(0)), 0);
    let hi = py::int(py::get_or(summary, "high_return_cubes", &json!(0)), 0);
    let score_19 = (5 + (xq_total / 5).min(3) + hi.min(2)).min(10);
    out.insert(
        "19_contests".into(),
        json!({
            "score": score_19,
            "weight": 4,
            "label": format!("雪球 {xq_total} 个组合持有 · {hi} 个收益 >50%"),
            "reasons_pass": if xq_total != 0 {
                vec![format!("{xq_total} 个雪球组合持有")]
            } else {
                Vec::<String>::new()
            },
        }),
    );

    // ── 加权总分 ──────────────────────────────────────────────
    let mut total_weighted: i64 = 0;
    let mut total_weight: i64 = 0;
    for v in out.values() {
        let score = py::int(py::get(v, "score"), 0);
        let weight = py::int(py::get(v, "weight"), 0);
        total_weighted += score * weight;
        total_weight += weight;
    }
    let fundamental = if total_weight != 0 {
        total_weighted as f64 / total_weight as f64 * 10.0
    } else {
        0.0
    };

    Ok(json!({
        "ticker": ticker,
        "fundamental_score": py::round_to(fundamental, 1),
        "dimensions": Value::Object(out),
    }))
}

/// Python truthiness for the JSON kinds the scorer tests with `if x:` / `or`.
fn is_falsy(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Bool(b) => !*b,
        Value::Number(n) => n.as_f64().map(|x| x == 0.0).unwrap_or(false),
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal_raw() -> Value {
        json!({ "ticker": "TEST", "dimensions": {} })
    }

    /// With no dimension data at all, every dimension must fall back to its
    /// documented default rather than panicking — this is the shape the
    /// reference run hit for its eight missing fetchers.
    #[test]
    fn missing_dimensions_yield_their_stub_defaults() {
        let scored = score_dimensions(&minimal_raw()).unwrap();
        let dims = &scored["dimensions"];
        assert_eq!(dims["3_macro"]["score"], 6);
        assert_eq!(dims["7_industry"]["score"], 7);
        assert_eq!(dims["18_trap"]["score"], 9);
        assert_eq!(dims["10_valuation"]["score"], 5);
        assert_eq!(
            dims["10_valuation"]["label"],
            "PE — · 5 年 50 分位 · 行业均值 —"
        );
    }

    /// The stub dimensions must not carry `reasons_*`, and dim 16/19 must omit
    /// `reasons_fail` specifically: key presence is part of the contract.
    #[test]
    fn reason_key_presence_matches_the_reference_shape() {
        let scored = score_dimensions(&minimal_raw()).unwrap();
        let dims = &scored["dimensions"];
        assert!(dims["3_macro"].get("reasons_pass").is_none());
        assert!(dims["1_financials"].get("reasons_pass").is_some());
        assert!(dims["16_lhb"].get("reasons_pass").is_some());
        assert!(dims["16_lhb"].get("reasons_fail").is_none());
        assert!(dims["19_contests"].get("reasons_fail").is_none());
        assert!(dims["15_events"].get("reasons_pass").is_none());
    }

    /// Weighted mean over the 19 dimensions, times ten, rounded to 1 dp.
    #[test]
    fn fundamental_score_is_the_weighted_mean_scaled_by_ten() {
        let scored = score_dimensions(&minimal_raw()).unwrap();
        let dims = scored["dimensions"].as_object().unwrap();
        let num: i64 = dims
            .values()
            .map(|v| v["score"].as_i64().unwrap() * v["weight"].as_i64().unwrap())
            .sum();
        let den: i64 = dims.values().map(|v| v["weight"].as_i64().unwrap()).sum();
        let expected = py::round_to(num as f64 / den as f64 * 10.0, 1);
        assert_eq!(scored["fundamental_score"].as_f64().unwrap(), expected);
        assert_eq!(den, 71, "reference weight total");
    }

    /// `"多头" in ma_align` also matches `"非多头"`; the reference run scored 4
    /// for a Stage 4 drawdown precisely because of this. Preserved on purpose.
    #[test]
    fn non_bullish_alignment_still_scores_the_bullish_bonus() {
        let raw = json!({
            "ticker": "TEST",
            "dimensions": { "2_kline": { "data": {
                "stage": "Stage 4 下跌", "ma_align": "非多头",
                "kline_stats": { "ytd_return": "-7.5%" }
            }}}
        });
        let scored = score_dimensions(&raw).unwrap();
        // 5 - 2 (Stage 4) + 1 ("非多头" contains "多头") = 4
        assert_eq!(scored["dimensions"]["2_kline"]["score"], 4);
        assert_eq!(
            scored["dimensions"]["2_kline"]["label"],
            "Stage 4 下跌 · 均线非多头 · YTD -7.5%"
        );
    }

    /// `growth` needs two revenue periods; a single period reads as 0%, which
    /// the reference run then reported as "停滞".
    #[test]
    fn revenue_growth_needs_two_periods() {
        let one = json!({
            "ticker": "TEST",
            "dimensions": { "1_financials": { "data": { "revenue_history": [100] } } }
        });
        let scored = score_dimensions(&one).unwrap();
        assert!(
            scored["dimensions"]["1_financials"]["label"]
                .as_str()
                .unwrap()
                .contains("营收增速 +0.0%")
        );

        let two = json!({
            "ticker": "TEST",
            "dimensions": { "1_financials": { "data": { "revenue_history": [100, 130] } } }
        });
        let scored = score_dimensions(&two).unwrap();
        assert!(
            scored["dimensions"]["1_financials"]["label"]
                .as_str()
                .unwrap()
                .contains("营收增速 +30.0%")
        );
    }

    #[test]
    fn pe_quantile_drives_the_valuation_band() {
        for (quantile, expected) in [("10 分位", 9), ("40", 7), ("60", 5), ("80", 3), ("95", 2)] {
            let raw = json!({
                "ticker": "TEST",
                "dimensions": { "10_valuation": { "data": { "pe_quantile": quantile } } }
            });
            let scored = score_dimensions(&raw).unwrap();
            assert_eq!(
                scored["dimensions"]["10_valuation"]["score"], expected,
                "quantile {quantile}"
            );
        }
    }

    #[test]
    fn missing_ticker_is_an_error() {
        assert!(score_dimensions(&json!({ "dimensions": {} })).is_err());
    }
}
