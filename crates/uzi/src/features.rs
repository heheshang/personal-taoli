//! Port of `stock_features.extract_features` (UZI-Skill v3.9.4).
//!
//! Produces the flat feature vector (~185 keys) that the investor rules are
//! evaluated against. This module is arithmetic over tables extracted from the
//! Python source by `tools/compile_panel_data.py`; the keyword lists and tier
//! maps stay data so they cannot drift by hand-transcription.
//!
//! Note this module needs a *different* number parser from `py::f`: the
//! original `stock_features._f` strips `¥` and `亿`, and treats a small set of
//! sentinel strings as missing. `score_fns._f` — used by `score.rs` — does not.
//! Two same-named functions with different behaviour is exactly the kind of
//! detail a port gets wrong, so both live here under distinct names.

use serde_json::{Map, Value, json};

use crate::panel_data::FeatureTables;
use crate::py::{get, round_to};

/// `stock_features._f(v, default=0.0)`.
///
/// Drops `,`, `%`, `+`, `¥` and `亿`, and maps a set of "no data" sentinels to
/// the default — unlike the scorer's `_f`, which would raise `ValueError` and
/// fall back instead.
pub fn f(v: &Value, default: Option<f64>) -> Option<f64> {
    // `str(v)` for the scalar kinds that can arrive here.
    let s: String = match v {
        Value::Null => return default,
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => (if *b { "True" } else { "False" }).to_string(),
        other => other.to_string(),
    };
    let s: String = s
        .trim()
        .chars()
        .filter(|c| !matches!(c, ',' | '%' | '+' | '¥' | '亿'))
        .collect();
    if s.is_empty() || matches!(s.as_str(), "-" | "—" | "None" | "nan" | "N/A") {
        return default;
    }
    match s.parse::<f64>() {
        Ok(x) => Some(x),
        Err(_) => default,
    }
}

/// Non-optional variant: missing input becomes `0.0`.
fn f0(v: &Value) -> f64 {
    f(v, None).unwrap_or(0.0)
}

/// `f0` with an explicit default.
fn fd(v: &Value, default: f64) -> f64 {
    f(v, Some(default)).unwrap_or(default)
}

/// `_avg` — mean of the non-zero values.
fn avg(values: &[Value], default: f64) -> f64 {
    let vals: Vec<f64> = values.iter().map(f0).filter(|x| *x != 0.0).collect();
    if vals.is_empty() {
        default
    } else {
        vals.iter().sum::<f64>() / vals.len() as f64
    }
}

/// `_min` — smallest non-zero value.
fn min_nonzero(values: &[Value], default: f64) -> f64 {
    values
        .iter()
        .map(f0)
        .filter(|x| *x != 0.0)
        .fold(None, |acc: Option<f64>, x| {
            Some(acc.map_or(x, |a| a.min(x)))
        })
        .unwrap_or(default)
}

/// `_last(values, default)`.
fn last(values: &[Value], default: f64) -> f64 {
    match values.last() {
        Some(v) => fd(v, default),
        None => default,
    }
}

/// `_pct_change(values, n)` — `n`-period change between first and last.
fn pct_change(values: &[Value], n: usize) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let last_v = f0(&values[values.len() - 1]);
    let earlier = if values.len() > n {
        f0(&values[values.len() - 1 - n])
    } else {
        f0(&values[0])
    };
    if earlier == 0.0 {
        return 0.0;
    }
    (last_v - earlier) / earlier.abs() * 100.0
}

/// `_market_cap_to_yi` — normalise raw yuan to 亿.
fn market_cap_to_yi(v: &Value, default: f64) -> f64 {
    let n = fd(v, default);
    if n > 1_000_000.0 { n / 1e8 } else { n }
}

/// `(dims.get(key) or {}).get("data") or {}`, but returning an empty map so the
/// many `.get()` reads below stay simple.
fn dd<'a>(raw: &'a Value, key: &str) -> &'a Value {
    let dim = get(get(raw, "dimensions"), key);
    let payload = get(dim, "data");
    if payload.is_object() { payload } else { &NULL }
}

static NULL: Value = Value::Null;

/// Python truthiness.
fn truthy(v: &Value) -> bool {
    crate::rules::truthy(v)
}

fn arr(v: &Value) -> &[Value] {
    v.as_array().map(|a| a.as_slice()).unwrap_or(&[])
}

fn first_digits(s: &str) -> Option<i64> {
    crate::py::first_digits(s)
}

/// Python's `round(x, n)`.
fn round_to_n(x: f64, digits: usize) -> f64 {
    round_to(x, digits)
}

/// Builds the feature vector for one stock.
pub fn extract_features(raw: &Value, tables: &FeatureTables) -> Value {
    let mut feat = Map::new();

    let basic = dd(raw, "0_basic");
    let fin = dd(raw, "1_financials");
    let kline = dd(raw, "2_kline");
    let macro_d = dd(raw, "3_macro");
    let peers = dd(raw, "4_peers");
    let chain = dd(raw, "5_chain");
    let research = dd(raw, "6_research");
    let industry = dd(raw, "7_industry");
    let valuation = dd(raw, "10_valuation");
    let gov = dd(raw, "11_governance");
    let capital = dd(raw, "12_capital_flow");
    let policy = dd(raw, "13_policy");
    let moat = dd(raw, "14_moat");
    let events = dd(raw, "15_events");
    let lhb = dd(raw, "16_lhb");
    let sentiment = dd(raw, "17_sentiment");
    let trap = dd(raw, "18_trap");
    let contests = dd(raw, "19_contests");

    // ── basic / price ────────────────────────────────────────────────
    set(
        &mut feat,
        "code",
        or_else(get(basic, "code"), get(raw, "ticker")),
    );
    set(&mut feat, "name", or_default(get(basic, "name"), "—"));
    set(
        &mut feat,
        "industry",
        or_default(get(basic, "industry"), "—"),
    );
    set(&mut feat, "price", json!(f0(get(basic, "price"))));
    set(&mut feat, "change_pct", json!(f0(get(basic, "change_pct"))));
    set(
        &mut feat,
        "market_cap_yi",
        json!(market_cap_to_yi(
            &or_else(get(basic, "market_cap_yi"), get(basic, "market_cap")),
            0.0
        )),
    );
    set(
        &mut feat,
        "circulating_cap_yi",
        json!(market_cap_to_yi(
            &or_else(
                get(basic, "circulating_cap_yi"),
                get(basic, "circulating_cap")
            ),
            0.0
        )),
    );
    set(
        &mut feat,
        "listed_date",
        json!(truncate(
            &py_str(get_or(basic, "listed_date", &json!(""))),
            10
        )),
    );
    set(
        &mut feat,
        "chairman",
        or_default(get(basic, "chairman"), "—"),
    );
    set(
        &mut feat,
        "actual_controller",
        or_default(get(basic, "actual_controller"), "—"),
    );
    set(&mut feat, "staff_num", json!(f0(get(basic, "staff_num"))));

    // ── financials ───────────────────────────────────────────────────
    let roe_hist = arr(get(fin, "roe_history"));
    let rev_hist = arr(get(fin, "revenue_history"));
    let np_hist = arr(get(fin, "net_profit_history"));
    let div_years = arr(get(fin, "dividend_years"));
    let div_amounts = arr(get(fin, "dividend_amounts"));

    set(&mut feat, "roe_latest", json!(last(roe_hist, 0.0)));
    set(
        &mut feat,
        "roe_5y_avg",
        json!(if roe_hist.len() >= 2 {
            avg(&roe_hist[roe_hist.len().saturating_sub(5)..], 0.0)
        } else {
            last(roe_hist, 0.0)
        }),
    );
    set(
        &mut feat,
        "roe_5y_min",
        json!(if roe_hist.len() >= 2 {
            min_nonzero(&roe_hist[roe_hist.len().saturating_sub(5)..], 0.0)
        } else {
            last(roe_hist, 0.0)
        }),
    );
    let tail5 = &roe_hist[roe_hist.len().saturating_sub(5)..];
    set(
        &mut feat,
        "roe_5y_above_15",
        json!(tail5.iter().filter(|v| f0(v) > 15.0).count()),
    );
    set(
        &mut feat,
        "roe_5y_above_10",
        json!(tail5.iter().filter(|v| f0(v) > 10.0).count()),
    );
    set(
        &mut feat,
        "roe_trend_up",
        json!(if roe_hist.len() >= 3 {
            last(roe_hist, 0.0) > avg(&roe_hist[..roe_hist.len() - 1], 0.0)
        } else {
            false
        }),
    );

    // `_f(x, None)` then a finiteness filter: non-finite becomes "absent".
    let revenue_ttm = f(get(fin, "revenue_ttm"), None).filter(|x| x.is_finite());
    let profit_ttm = f(get(fin, "net_profit_ttm"), None).filter(|x| x.is_finite());
    set(
        &mut feat,
        "revenue_latest_yi",
        json!(revenue_ttm.unwrap_or_else(|| last(rev_hist, 0.0))),
    );
    set(
        &mut feat,
        "revenue_latest_basis",
        json!(if revenue_ttm.is_some() {
            "ttm"
        } else {
            "annual"
        }),
    );
    let explicit_rev_yoy = get(fin, "revenue_growth_yoy");
    set(
        &mut feat,
        "revenue_growth_latest",
        json!(if !explicit_rev_yoy.is_null() {
            f0(explicit_rev_yoy)
        } else {
            pct_change(rev_hist, 1)
        }),
    );
    set(
        &mut feat,
        "revenue_growth_period",
        get(fin, "revenue_growth_period").clone(),
    );
    set(
        &mut feat,
        "revenue_growth_basis",
        get(fin, "revenue_growth_basis").clone(),
    );
    set(
        &mut feat,
        "revenue_growth_source",
        get(fin, "revenue_growth_source").clone(),
    );
    set(
        &mut feat,
        "revenue_growth_3y_cagr",
        json!(
            if rev_hist.len() >= 4 && f0(&rev_hist[rev_hist.len() - 4]) > 0.0 {
                ((last(rev_hist, 0.0) / f0(&rev_hist[rev_hist.len() - 4])).powf(1.0 / 3.0) - 1.0)
                    * 100.0
            } else {
                0.0
            }
        ),
    );

    set(
        &mut feat,
        "net_profit_latest_yi",
        json!(profit_ttm.unwrap_or_else(|| last(np_hist, 0.0))),
    );
    set(
        &mut feat,
        "net_profit_latest_basis",
        json!(if profit_ttm.is_some() {
            "ttm"
        } else {
            "annual"
        }),
    );
    let explicit_profit_yoy = get(fin, "net_profit_growth_yoy");
    set(
        &mut feat,
        "net_profit_growth_latest",
        json!(if !explicit_profit_yoy.is_null() {
            f0(explicit_profit_yoy)
        } else {
            pct_change(np_hist, 1)
        }),
    );
    set(
        &mut feat,
        "net_profit_growth_period",
        get(fin, "net_profit_growth_period").clone(),
    );
    set(
        &mut feat,
        "net_profit_growth_basis",
        get(fin, "net_profit_growth_basis").clone(),
    );
    set(
        &mut feat,
        "net_profit_growth_source",
        get(fin, "net_profit_growth_source").clone(),
    );
    set(
        &mut feat,
        "net_profit_5y_positive",
        json!(
            np_hist[np_hist.len().saturating_sub(5)..]
                .iter()
                .filter(|v| f0(v) > 0.0)
                .count()
        ),
    );
    set(
        &mut feat,
        "consecutive_profit_years",
        json!(np_hist.iter().filter(|v| f0(v) > 0.0).count()),
    );

    // Ratio needs a matching period; a single usable pairing is enough.
    let (margin_rev, margin_profit, margin_basis) =
        if let (Some(r), Some(p)) = (revenue_ttm, profit_ttm) {
            (Some(r), Some(p), "ttm")
        } else if !rev_hist.is_empty() && rev_hist.len() == np_hist.len() {
            (
                f(rev_hist.last().unwrap(), None),
                f(np_hist.last().unwrap(), None),
                "annual",
            )
        } else {
            (None, None, "unavailable")
        };
    if let (Some(r), Some(p)) = (margin_rev, margin_profit)
        && r > 0.0
    {
        set(&mut feat, "net_margin", json!(round_to_n(p / r * 100.0, 1)));
        set(&mut feat, "net_margin_basis", json!(margin_basis));
    } else {
        let reported = f(get(fin, "net_margin"), None);
        set(
            &mut feat,
            "net_margin",
            reported.map(|x| json!(x)).unwrap_or(Value::Null),
        );
        set(
            &mut feat,
            "net_margin_basis",
            json!(if reported.is_some() {
                "reported"
            } else {
                "unavailable"
            }),
        );
    }

    let health = get(fin, "financial_health");
    set(
        &mut feat,
        "current_ratio",
        json!(f0(get(health, "current_ratio"))),
    );
    set(
        &mut feat,
        "debt_ratio",
        json!(f0(get(health, "debt_ratio"))),
    );
    set(
        &mut feat,
        "fcf_margin",
        json!(f0(get(health, "fcf_margin"))),
    );
    set(
        &mut feat,
        "ocf_to_net_income_ratio",
        json!(fd(
            &or_else(
                get(fin, "ocf_to_net_income_ratio"),
                get(health, "ocf_to_net_income_ratio")
            ),
            0.0
        )),
    );
    set(&mut feat, "roic", json!(f0(get(health, "roic"))));

    let dupont = get(fin, "dupont");
    if truthy(dupont) {
        set(
            &mut feat,
            "dupont_net_margin",
            json!(f0(get(dupont, "net_margin_pct"))),
        );
        set(
            &mut feat,
            "dupont_asset_turnover",
            json!(f0(get(dupont, "asset_turnover"))),
        );
        set(
            &mut feat,
            "dupont_equity_multiplier",
            json!(f0(get(dupont, "equity_multiplier"))),
        );
        set(
            &mut feat,
            "dupont_roe",
            json!(f0(get(dupont, "roe_reconstructed_pct"))),
        );
        set(
            &mut feat,
            "roe_quality",
            json!(py_str(get(dupont, "roe_quality"))),
        );
    }

    set(
        &mut feat,
        "consecutive_dividend_years",
        json!(div_years.len()),
    );
    set(
        &mut feat,
        "dividend_yield",
        json!(f0(get(basic, "dividend_yield_ttm"))),
    );
    set(
        &mut feat,
        "total_dividend_5y_per_10",
        json!(
            div_amounts[div_amounts.len().saturating_sub(5)..]
                .iter()
                .map(f0)
                .sum::<f64>()
        ),
    );

    // ── k-line / technical ───────────────────────────────────────────
    let stage = py_str(get_or(kline, "stage", &json!("—")));
    let stage_num = if stage.contains("Stage 2") {
        2
    } else if stage.contains("Stage 1") {
        1
    } else if stage.contains("Stage 3") {
        3
    } else if stage.contains("Stage 4") {
        4
    } else {
        0
    };
    set(&mut feat, "stage", json!(stage));
    set(&mut feat, "stage_num", json!(stage_num));
    let ma_align = py_str(get_or(kline, "ma_align", &json!("—")));
    set(&mut feat, "ma_align", json!(ma_align));
    set(
        &mut feat,
        "ma_bull_aligned",
        json!(ma_align.contains("多头")),
    );
    let macd = py_str(get_or(kline, "macd", &json!("—")));
    set(&mut feat, "macd", json!(macd));
    set(
        &mut feat,
        "macd_golden_cross",
        json!(macd.contains("金叉") && macd.contains("水上")),
    );
    let rsi = f0(get(kline, "rsi"));
    set(&mut feat, "rsi", json!(rsi));
    set(&mut feat, "rsi_overbought", json!(rsi > 70.0));
    set(&mut feat, "rsi_oversold", json!(rsi < 30.0));

    let ind = get(kline, "indicators");
    set(&mut feat, "kdj_k", json!(f0(get(ind, "kdj_k"))));
    set(&mut feat, "kdj_d", json!(f0(get(ind, "kdj_d"))));
    set(&mut feat, "kdj_j", json!(f0(get(ind, "kdj_j"))));
    set(
        &mut feat,
        "kdj_golden_cross",
        json!(
            truthy(get(ind, "kdj_k"))
                && truthy(get(ind, "kdj_d"))
                && f0(get(ind, "kdj_k")) > f0(get(ind, "kdj_d"))
        ),
    );
    set(
        &mut feat,
        "obv_trend_up",
        json!(truthy(get(ind, "obv_trend_up"))),
    );
    let williams = f0(get(ind, "williams_r"));
    set(&mut feat, "williams_r", json!(williams));
    set(
        &mut feat,
        "williams_overbought",
        json!(if get(ind, "williams_r").is_null() {
            false
        } else {
            williams > -20.0
        }),
    );
    set(
        &mut feat,
        "williams_oversold",
        json!(if get(ind, "williams_r").is_null() {
            false
        } else {
            williams < -80.0
        }),
    );

    let stats = get(kline, "kline_stats");
    set(&mut feat, "ytd_return", json!(f0(get(stats, "ytd_return"))));
    set(
        &mut feat,
        "volatility_1y",
        json!(f0(get(stats, "volatility"))),
    );
    set(
        &mut feat,
        "max_drawdown_1y",
        json!(f0(get(stats, "max_drawdown"))),
    );

    let candles = arr(get(kline, "candles_60d"));
    if !candles.is_empty() {
        let closes: Vec<f64> = candles.iter().map(|c| f0(get(c, "close"))).collect();
        let highs: Vec<f64> = candles.iter().map(|c| f0(get(c, "high"))).collect();
        let lows: Vec<f64> = candles.iter().map(|c| f0(get(c, "low"))).collect();
        if !closes.is_empty() && !highs.is_empty() && !lows.is_empty() {
            let (hi, lo) = (
                highs.iter().cloned().fold(f64::MIN, f64::max),
                lows.iter().cloned().fold(f64::MAX, f64::min),
            );
            let c_last = closes[closes.len() - 1];
            set(
                &mut feat,
                "pct_from_60d_high",
                json!(if hi > 0.0 {
                    (c_last - hi) / hi * 100.0
                } else {
                    0.0
                }),
            );
            set(
                &mut feat,
                "pct_from_60d_low",
                json!(if lo > 0.0 {
                    (c_last - lo) / lo * 100.0
                } else {
                    0.0
                }),
            );
        }
    }

    set(&mut feat, "vcp_hint", json!(false));

    // ── valuation ────────────────────────────────────────────────────
    let pe = {
        let a = f0(get(basic, "pe_ttm"));
        if a != 0.0 {
            a
        } else {
            f0(get(valuation, "pe"))
        }
    };
    let pb = {
        let a = f0(get(basic, "pb"));
        if a != 0.0 {
            a
        } else {
            f0(get(valuation, "pb"))
        }
    };
    set(&mut feat, "pe", json!(pe));
    set(&mut feat, "pb", json!(pb));
    set(&mut feat, "pe_x_pb", json!(pe * pb));
    let pe_q = first_digits(&py_str(get_or(valuation, "pe_quantile", &json!("")))).unwrap_or(50);
    set(&mut feat, "pe_quantile_5y", json!(pe_q));
    let industry_pe = f0(get(valuation, "industry_pe"));
    set(&mut feat, "industry_pe", json!(industry_pe));
    set(
        &mut feat,
        "pe_vs_industry",
        json!(if industry_pe > 0.0 {
            (pe - industry_pe) / industry_pe * 100.0
        } else {
            0.0
        }),
    );
    let dcf_intrinsic = first_decimal(&py_str(get_or(valuation, "dcf", &json!("")))).unwrap_or(0.0);
    set(&mut feat, "dcf_intrinsic_yi", json!(dcf_intrinsic));
    let mcap = f0(feat.get("market_cap_yi").unwrap_or(&Value::Null));
    set(
        &mut feat,
        "safety_margin",
        json!(if mcap > 0.0 {
            (dcf_intrinsic - mcap) / mcap * 100.0
        } else {
            0.0
        }),
    );

    // ── peers ────────────────────────────────────────────────────────
    let peer_table = arr(get(peers, "peer_table"));
    let peer_pes: Vec<f64> = peer_table
        .iter()
        .filter(|p| !truthy(get(p, "is_self")))
        .map(|p| f0(get(p, "pe")))
        .filter(|x| *x > 0.0)
        .collect();
    set(&mut feat, "peers_count", json!(peer_table.len()));
    let peer_avg = if peer_pes.is_empty() {
        0.0
    } else {
        peer_pes.iter().sum::<f64>() / peer_pes.len() as f64
    };
    set(&mut feat, "peer_avg_pe", json!(peer_avg));
    set(
        &mut feat,
        "vs_peer_avg_pe",
        json!(if peer_avg > 0.0 {
            (pe - peer_avg) / peer_avg * 100.0
        } else {
            0.0
        }),
    );
    set(&mut feat, "is_industry_leader", json!(false));
    if !peer_table.is_empty() {
        let self_idx = peer_table
            .iter()
            .position(|p| truthy(get(p, "is_self")))
            .map(|i| i as i64)
            .unwrap_or(-1);
        set(
            &mut feat,
            "industry_rank",
            json!(if self_idx >= 0 { self_idx + 1 } else { 0 }),
        );
    }

    // ── research ─────────────────────────────────────────────────────
    let coverage = {
        let a = f0(get(research, "coverage_count"));
        if a != 0.0 {
            a
        } else {
            f0(get(research, "report_count"))
        }
    };
    set(&mut feat, "research_coverage", json!(coverage));
    set(
        &mut feat,
        "buy_rating_pct",
        json!(f0(get(research, "buy_rating_pct"))),
    );
    let target_avg = f0(get(research, "target_price_avg"));
    set(&mut feat, "target_price_avg", json!(target_avg));
    let consensus_eps = f0(get(research, "consensus_eps_2026"));
    set(&mut feat, "consensus_eps_2026", json!(consensus_eps));
    set(
        &mut feat,
        "consensus_pe_2026",
        json!(f0(get(research, "consensus_pe_2026"))),
    );
    set(
        &mut feat,
        "upside_to_target",
        json!(if pe > 0.0 && target_avg > 0.0 {
            (target_avg - f0(get(basic, "price"))) / f0(get(basic, "price")) * 100.0
        } else {
            0.0
        }),
    );
    let eps_latest = f0(get(basic, "eps"));
    set(
        &mut feat,
        "consensus_growth_to_2026",
        json!(if eps_latest > 0.0 && consensus_eps > 0.0 {
            (consensus_eps / eps_latest - 1.0) * 100.0
        } else {
            0.0
        }),
    );

    // ── industry ─────────────────────────────────────────────────────
    set(
        &mut feat,
        "industry_growth_pct",
        json!(f0(get(industry, "growth"))),
    );
    let lifecycle = py_str(get_or(industry, "lifecycle", &json!("—")));
    set(
        &mut feat,
        "industry_is_growing",
        json!(lifecycle.contains("成长")),
    );
    set(
        &mut feat,
        "industry_in_decline",
        json!(lifecycle.contains("衰退")),
    );
    set(&mut feat, "industry_lifecycle", json!(lifecycle));

    // ── capital flow ─────────────────────────────────────────────────
    let main_flow = arr(get(capital, "main_fund_flow_20d"));
    let mut main_5d = 0.0_f64;
    for rec in main_flow.iter().take(5) {
        let raw_v = if rec.is_object() {
            get(rec, "主力净流入-净额")
        } else {
            &Value::Null
        };
        let raw_v = if raw_v.is_null() { &Value::Null } else { raw_v };
        match raw_v {
            Value::Number(n) => main_5d += n.as_f64().unwrap_or(0.0),
            Value::String(s) => {
                if let Ok(x) = s.trim().parse::<f64>() {
                    main_5d += x;
                }
            }
            Value::Bool(true) => main_5d += 1.0,
            Value::Bool(false) => {}
            _ => {}
        }
    }
    set(
        &mut feat,
        "main_fund_5d_net_yi",
        json!(round_to_n(main_5d / 1e8, 2)),
    );
    set(&mut feat, "main_fund_net_positive", json!(main_5d > 0.0));
    set(
        &mut feat,
        "northbound_20d_yi",
        json!(round_to_n(main_5d / 1e8, 2)),
    );
    set(&mut feat, "northbound_net_positive", json!(main_5d > 0.0));
    let margin_trend = py_str(get_or(capital, "margin_trend", &json!("—")));
    let holders_trend = py_str(get_or(capital, "holders_trend", &json!("—")));
    set(&mut feat, "margin_trend", json!(margin_trend));
    set(
        &mut feat,
        "holders_concentrating",
        json!(holders_trend.contains("降")),
    );
    set(&mut feat, "holders_trend", json!(holders_trend));
    set(
        &mut feat,
        "unlock_pressure_12m",
        json!(arr(get(capital, "unlock_schedule")).len()),
    );

    // ── governance ───────────────────────────────────────────────────
    let pledge = arr(get(gov, "pledge"));
    set(
        &mut feat,
        "has_pledge_issue",
        json!(
            !pledge.is_empty()
                && pledge
                    .iter()
                    .any(|p| p.is_object() && f0(get(p, "质押比例")) > 30.0)
        ),
    );
    set(
        &mut feat,
        "insider_net_buy",
        json!(!arr(get(gov, "insider_trades_1y")).is_empty()),
    );
    set(&mut feat, "no_violations", json!(true));

    // ── moat ─────────────────────────────────────────────────────────
    let moat_scores = get(moat, "scores");
    let moat_known = truthy(moat_scores);
    set(&mut feat, "moat_known", json!(moat_known));
    let moat_field = |key: &str| -> Value {
        if moat_known {
            json!(f0(get(moat_scores, key)))
        } else {
            Value::Null
        }
    };
    let mi = moat_field("intangible");
    let ms = moat_field("switching");
    let mn = moat_field("network");
    let msc = moat_field("scale");
    set(&mut feat, "moat_intangible", mi.clone());
    set(&mut feat, "moat_switching", ms.clone());
    set(&mut feat, "moat_network", mn.clone());
    set(&mut feat, "moat_scale", msc.clone());
    let moat_total = if moat_known {
        json!(
            mi.as_f64().unwrap_or(0.0)
                + ms.as_f64().unwrap_or(0.0)
                + mn.as_f64().unwrap_or(0.0)
                + msc.as_f64().unwrap_or(0.0)
        )
    } else {
        Value::Null
    };
    set(&mut feat, "moat_total", moat_total.clone());
    set(
        &mut feat,
        "moat_clear",
        if moat_known {
            json!(moat_total.as_f64().unwrap_or(0.0) >= 24.0)
        } else {
            Value::Null
        },
    );

    // ── events ───────────────────────────────────────────────────────
    let timeline: Vec<String> = arr(get(events, "event_timeline"))
        .iter()
        .map(py_str)
        .collect();
    set(&mut feat, "recent_events_count", json!(timeline.len()));
    let text = timeline.join(" ").to_lowercase();
    set(
        &mut feat,
        "has_positive_catalyst",
        json!(
            ["预告", "增长", "大订单", "新品", "合作", "并购"]
                .iter()
                .any(|k| text.contains(k))
        ),
    );
    set(
        &mut feat,
        "has_negative_catalyst",
        json!(
            ["亏损", "下修", "处罚", "诉讼", "风险"]
                .iter()
                .any(|k| text.contains(k))
        ),
    );

    // ── lhb ──────────────────────────────────────────────────────────
    set(
        &mut feat,
        "lhb_30d_count",
        json!(f0(get(lhb, "lhb_count_30d"))),
    );
    let matched: Vec<Value> = arr(get(lhb, "matched_youzi")).to_vec();
    set(&mut feat, "matched_youzi_count", json!(matched.len()));
    set(&mut feat, "matched_youzi", Value::Array(matched));
    let inst_vs = get(lhb, "inst_vs_youzi");
    set(
        &mut feat,
        "inst_net_buy_lhb",
        json!(f0(get(inst_vs, "institutional_net"))),
    );
    set(
        &mut feat,
        "youzi_net_buy_lhb",
        json!(f0(get(inst_vs, "youzi_net"))),
    );

    // ── sentiment ────────────────────────────────────────────────────
    set(
        &mut feat,
        "sentiment_heat",
        json!(f0(get(sentiment, "thermometer_value"))),
    );
    set(
        &mut feat,
        "sentiment_positive_pct",
        json!(f0(get(sentiment, "positive_pct"))),
    );
    set(
        &mut feat,
        "sentiment_label",
        json!(py_str(get_or(sentiment, "sentiment_label", &json!("中性")))),
    );

    // ── trap ─────────────────────────────────────────────────────────
    let trap_hits = {
        let a = f0(get(trap, "signals_hit_count"));
        if a != 0.0 { a } else { 0.0 }
    };
    set(&mut feat, "trap_signals_hit", json!(trap_hits));
    let trap_level = py_str(get_or(trap, "trap_level", &json!("🟢 安全")));
    set(&mut feat, "is_safe", json!(trap_level.contains("安全")));
    set(&mut feat, "trap_level", json!(trap_level));

    // ── contests ─────────────────────────────────────────────────────
    let summary = get(contests, "summary");
    set(
        &mut feat,
        "xq_cube_count",
        json!(f0(get(summary, "xueqiu_cubes_total"))),
    );
    set(
        &mut feat,
        "xq_high_return_count",
        json!(f0(get(summary, "high_return_cubes"))),
    );

    // ── fund managers ────────────────────────────────────────────────
    let fms = arr(get(raw, "fund_managers"));
    set(&mut feat, "fund_manager_count", json!(fms.len()));
    if !fms.is_empty() {
        let returns: Vec<f64> = fms.iter().map(|m| f0(get(m, "return_5y"))).collect();
        let max_ret = returns.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        set(
            &mut feat,
            "fund_manager_max_5y_return",
            json!(if returns.is_empty() { 0.0 } else { max_ret }),
        );
        set(
            &mut feat,
            "has_top_fund_holder",
            json!(fms.iter().any(|m| f0(get(m, "return_5y")) > 100.0)),
        );
    } else {
        set(&mut feat, "fund_manager_max_5y_return", json!(0.0));
        set(&mut feat, "has_top_fund_holder", json!(false));
    }

    // ── macro ────────────────────────────────────────────────────────
    let rate_cycle = py_str(get_or(macro_d, "rate_cycle", &json!("中性")));
    set(
        &mut feat,
        "macro_rate_easing",
        json!(
            ["利好", "降息", "宽松"]
                .iter()
                .any(|k| rate_cycle.contains(k))
        ),
    );
    set(&mut feat, "macro_rate_cycle", json!(rate_cycle));
    set(
        &mut feat,
        "macro_commodity",
        json!(py_str(get_or(macro_d, "commodity", &json!("中性")))),
    );

    // ── policy ───────────────────────────────────────────────────────
    let policy_dir = py_str(get_or(policy, "policy_dir", &json!("")));
    set(
        &mut feat,
        "policy_supportive",
        json!(policy_dir.contains("积极")),
    );
    set(
        &mut feat,
        "policy_tightening",
        json!(policy_dir.contains("收紧")),
    );

    // ── fin-model support ────────────────────────────────────────────
    let mcap2 = f0(feat.get("market_cap_yi").unwrap_or(&Value::Null));
    let px = f0(feat.get("price").unwrap_or(&Value::Null));
    let shares = if px > 0.0 {
        round_to_n(mcap2 / px, 3)
    } else {
        0.0
    };
    set(&mut feat, "shares_outstanding_yi", json!(shares));
    let latest_ni = last(arr(get(fin, "net_profit_history")), 0.0);
    set(
        &mut feat,
        "eps",
        json!(if shares > 0.0 {
            round_to_n(latest_ni / shares, 3)
        } else {
            0.0
        }),
    );
    let eq = f0(feat.get("equity_yi").unwrap_or(&Value::Null));
    set(
        &mut feat,
        "bvps",
        json!(if shares > 0.0 {
            round_to_n(eq / shares, 3)
        } else {
            0.0
        }),
    );
    let real_ocf_raw = get(fin, "operating_cash_flow_yi");
    let real_ocf = f0(real_ocf_raw);
    let fcf_known = !real_ocf_raw.is_null();
    set(&mut feat, "fcf_known", json!(fcf_known));
    if fcf_known {
        set(&mut feat, "fcf_latest_yi", json!(round_to_n(real_ocf, 2)));
    } else {
        set(
            &mut feat,
            "fcf_latest_yi",
            json!(if latest_ni > 0.0 {
                round_to_n(latest_ni * 0.8, 2)
            } else {
                0.0
            }),
        );
    }
    set(&mut feat, "fcf_is_proxy", json!(!fcf_known));
    set(
        &mut feat,
        "fcf_positive",
        if fcf_known {
            json!(real_ocf > 0.0)
        } else {
            Value::Null
        },
    );
    set(
        &mut feat,
        "ebitda_yi",
        json!(if latest_ni > 0.0 {
            round_to_n(latest_ni / 0.6, 2)
        } else {
            0.0
        }),
    );
    set(
        &mut feat,
        "total_debt_yi",
        json!(if health.is_object() {
            fd(get(health, "total_debt"), 0.0)
        } else {
            0.0
        }),
    );
    set(
        &mut feat,
        "cash_yi",
        json!(if health.is_object() {
            fd(get(health, "cash"), 0.0)
        } else {
            0.0
        }),
    );
    set(
        &mut feat,
        "equity_yi",
        json!(if health.is_object() {
            fd(get(health, "equity"), 0.0)
        } else {
            0.0
        }),
    );
    let gross = f0(get(fin, "gross_margin"));
    set(
        &mut feat,
        "gross_margin",
        json!(if gross != 0.0 { gross } else { 0.0 }),
    );
    let rev_latest = f0(feat.get("revenue_latest_yi").unwrap_or(&Value::Null));
    set(
        &mut feat,
        "ps",
        json!(if rev_latest > 0.0 {
            round_to_n(mcap2 / rev_latest, 2)
        } else {
            0.0
        }),
    );

    // industry growth: numeric passthrough, else first `%`-suffixed number.
    let growth_raw = get(industry, "growth");
    let industry_growth = match growth_raw {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => first_percent(s).unwrap_or(0.0),
        _ => 0.0,
    };
    set(&mut feat, "industry_growth", json!(industry_growth));

    let cmcap = market_cap_to_yi(
        &or_else(get(basic, "market_cap_yi"), get(basic, "market_cap")),
        0.0,
    );
    let imcap = f0(get(get(industry, "cninfo_metrics"), "total_mcap_yi"));
    set(
        &mut feat,
        "market_share",
        json!(if cmcap > 0.0 && imcap > 0.0 {
            round_to_n(cmcap / imcap * 100.0, 2)
        } else {
            0.0
        }),
    );
    let div_basic = f0(get(basic, "dividend_yield_ttm"));
    set(
        &mut feat,
        "dividend_yield",
        json!(if div_basic != 0.0 {
            div_basic
        } else {
            fd(get(valuation, "dividend_yield"), 0.0)
        }),
    );
    let g3y = f0(feat.get("revenue_growth_3y_cagr").unwrap_or(&Value::Null));
    set(
        &mut feat,
        "peg",
        json!(round_to_n(if g3y > 0.0 { pe / g3y } else { 99.0 }, 2)),
    );
    set(&mut feat, "gross_margin_expanding", json!(false));
    let ticker_str = {
        let t = get(raw, "ticker");
        if t.is_null() {
            String::new()
        } else {
            py_str(t)
        }
    };
    set(&mut feat, "ticker", json!(ticker_str));
    let market = if ticker_str.ends_with(".SZ") || ticker_str.ends_with(".SH") {
        "A"
    } else if ticker_str.ends_with(".HK") {
        "HK"
    } else {
        "US"
    };
    set(&mut feat, "market", json!(market));

    // ── AI chokepoint (Serenity / group H) ───────────────────────────
    let chain_txt = if truthy(chain) {
        compact_json(chain)
    } else {
        String::new()
    };
    let ind_txt = if industry.is_object() {
        compact_json(industry)
    } else {
        py_str(industry)
    };
    let mut blob_parts = vec![
        py_str(feat.get("industry").unwrap_or(&Value::Null)),
        py_str(feat.get("name").unwrap_or(&Value::Null)),
        chain_txt,
        ind_txt,
        text.clone(),
    ];
    blob_parts.retain(|s| !s.is_empty());
    let blob = blob_parts.join(" ").to_lowercase();

    let ai_hit: Vec<Value> = tables
        .ai_chokepoint_kw
        .iter()
        .filter(|kw| blob.contains(kw.as_str()))
        .take(8)
        .map(|s| json!(s))
        .collect();
    let ai_hit_len = ai_hit.len();
    set(&mut feat, "ai_chain_hit", json!(ai_hit_len > 0));
    set(&mut feat, "ai_chain_keywords", Value::Array(ai_hit));

    let irrepl = f0(feat.get("moat_switching").unwrap_or(&Value::Null))
        + f0(feat.get("moat_scale").unwrap_or(&Value::Null));
    set(&mut feat, "ai_irreplaceable", json!(irrepl >= 12.0));

    let mc = f0(feat.get("market_cap_yi").unwrap_or(&Value::Null));
    let elasticity = if mc <= 0.0 {
        0.5
    } else if mc < 100.0 {
        1.0
    } else if mc < 300.0 {
        0.8
    } else if mc < 800.0 {
        0.5
    } else if mc < 2000.0 {
        0.25
    } else {
        0.1
    };
    set(&mut feat, "ai_smallcap", json!(mc > 0.0 && mc < 300.0));

    let mut inflection: f64 = 0.0;
    if truthy(feat.get("policy_supportive").unwrap_or(&Value::Null)) {
        inflection += 0.4;
    }
    if truthy(feat.get("has_positive_catalyst").unwrap_or(&Value::Null)) {
        inflection += 0.3;
    }
    if f0(feat.get("industry_growth").unwrap_or(&Value::Null)) >= 20.0 {
        inflection += 0.3;
    }
    let inflection: f64 = inflection.min(1.0);

    // Supply-chain tier: first (most upstream) matching layer wins.
    let mut tier_name = "未分层".to_string();
    let mut tier_weight = 0.55;
    for layer in &tables.tier_map {
        if layer.keywords.iter().any(|k| blob.contains(k.as_str())) {
            tier_name = layer.name.clone();
            tier_weight = layer.weight;
            break;
        }
    }
    set(&mut feat, "ai_chain_tier", json!(tier_name));
    set(&mut feat, "ai_chain_tier_weight", json!(tier_weight));

    // Evidence ladder.
    let mut ev = 0;
    if truthy(feat.get("net_margin").unwrap_or(&Value::Null)) {
        ev += 1;
    }
    if truthy(feat.get("has_positive_catalyst").unwrap_or(&Value::Null)) {
        ev += 1;
    }
    if tables
        .hard_evidence_kw
        .iter()
        .any(|k| blob.contains(k.as_str()))
    {
        ev += 1;
    }
    let (ev_grade, ev_mult) = match ev {
        0 => ("weak", 0.70),
        1 => ("medium", 0.85),
        _ => ("strong", 1.0),
    };
    set(&mut feat, "ai_evidence_grade", json!(ev_grade));

    // Penalties: eight factors, total capped at 60%.
    let mut penalties = Map::new();
    let heat = f0(feat.get("sentiment_heat").unwrap_or(&Value::Null));
    let is_safe = !feat.get("is_safe").map(|v| !truthy(v)).unwrap_or(false);
    if truthy(feat.get("ai_chain_hit").unwrap_or(&Value::Null)) && heat >= 70.0 && ev == 0 {
        penalties.insert("hype_no_orders".into(), json!(0.30));
    }
    if mc > 0.0 && mc < 30.0 {
        penalties.insert("liquidity".into(), json!(0.20));
    } else if (30.0..50.0).contains(&mc) {
        penalties.insert("liquidity".into(), json!(0.10));
    }
    if !is_safe {
        penalties.insert("accounting_trap".into(), json!(0.25));
    }
    if truthy(feat.get("has_pledge_issue").unwrap_or(&Value::Null)) {
        penalties.insert("governance".into(), json!(0.15));
    }
    if [
        "钢铁",
        "煤炭",
        "有色冶炼",
        "化工原料",
        "航运",
        "水泥",
        "养殖",
        "周期",
    ]
    .iter()
    .any(|k| blob.contains(k))
    {
        penalties.insert("cyclicality".into(), json!(0.15));
    }
    if [
        "技术路线之争",
        "被替代",
        "替代风险",
        "路线分歧",
        "新技术冲击",
        "颠覆性替代",
    ]
    .iter()
    .any(|k| blob.contains(k))
    {
        penalties.insert("alt_design".into(), json!(0.15));
    }
    let has_export_control = ["出口管制", "制裁", "实体清单", "断供"]
        .iter()
        .any(|k| blob.contains(k));
    let has_domestic_sub = ["国产替代", "自主可控", "进口替代"]
        .iter()
        .any(|k| blob.contains(k));
    if has_export_control && !has_domestic_sub {
        penalties.insert("geopolitics".into(), json!(0.15));
    }
    if [
        "定增",
        "增发",
        "可转债",
        "再融资",
        "配股",
        "解禁",
        "股权激励摊薄",
    ]
    .iter()
    .any(|k| blob.contains(k))
    {
        penalties.insert("dilution".into(), json!(0.15));
    }
    let penalty_total: f64 = penalties
        .values()
        .map(|v| v.as_f64().unwrap_or(0.0))
        .sum::<f64>()
        .min(0.60);
    set(&mut feat, "ai_penalties", Value::Object(penalties));
    set(
        &mut feat,
        "ai_penalty_total",
        json!(round_to_n(penalty_total, 2)),
    );

    // Composite: chain membership is a gate, then tier x evidence x (1 - penalty).
    let chokepoint = if truthy(feat.get("ai_chain_hit").unwrap_or(&Value::Null)) {
        let kw_strength = (ai_hit_len.min(3) as f64) / 3.0;
        let irr_norm = (irrepl / 16.0).min(1.0);
        let base = 0.35 * kw_strength + 0.30 * irr_norm + 0.20 * elasticity + 0.15 * inflection;
        let base = base * (0.70 + 0.30 * tier_weight);
        let base = base * ev_mult;
        let base = base * (1.0 - penalty_total);
        base * 100.0
    } else {
        8.0 * elasticity
    };
    set(
        &mut feat,
        "ai_chokepoint_score",
        json!(round_to_n(chokepoint, 1)),
    );

    // Compatibility aliases (v3.9.4). The rule engine reads these names, but the
    // feature layer only ever produced the longer ones, so every referencing
    // rule was silently seeing 0 and failing.
    let v_pe_ttm = feat.get("pe").cloned().unwrap_or(json!(0));
    set(&mut feat, "pe_ttm", v_pe_ttm);
    let v_rev_growth_3y = feat
        .get("revenue_growth_3y_cagr")
        .cloned()
        .unwrap_or(json!(0));
    set(&mut feat, "rev_growth_3y", v_rev_growth_3y);
    let v_rev_growth_yoy = feat
        .get("revenue_growth_latest")
        .cloned()
        .unwrap_or(json!(0));
    set(&mut feat, "rev_growth_yoy", v_rev_growth_yoy);
    let v_roe = feat.get("roe_latest").cloned().unwrap_or(json!(0));
    set(&mut feat, "roe", v_roe);
    let v_net_profit_growth_3y = feat
        .get("net_profit_growth_latest")
        .cloned()
        .unwrap_or(json!(0));
    set(&mut feat, "net_profit_growth_3y", v_net_profit_growth_3y);

    // Keys that no fetcher can supply. Left as null so the rule engine treats
    // them as missing evidence (skip) rather than failing the rule outright.
    for key in &tables.no_data_keys {
        set(&mut feat, key, Value::Null);
    }

    Value::Object(feat)
}

/// Inserts one feature. A free function rather than a closure so the map can
/// be read between writes (several features derive from earlier ones).
fn set(feat: &mut Map<String, Value>, key: &str, value: Value) {
    feat.insert(key.to_string(), value);
}

/// `x or y` returning the first truthy operand (Python `or` semantics).
fn or_else(a: &Value, b: &Value) -> Value {
    if truthy(a) {
        a.clone()
    } else if truthy(b) {
        b.clone()
    } else {
        Value::Null
    }
}

fn or_default<'a>(v: &'a Value, default: &'a str) -> Value {
    if truthy(v) { v.clone() } else { json!(default) }
}

fn get_or<'a>(v: &'a Value, key: &str, default: &'a Value) -> &'a Value {
    crate::py::get_or(v, key, default)
}

fn py_str(v: &Value) -> String {
    // Python's `str()` on a dict/list is repr-like; the callers here only pass
    // scalars for `str(...)` and rely on json for the blob, so scalars suffice.
    crate::py::py_str(v)
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `json.dumps(obj, ensure_ascii=False)` with the default separators.
fn compact_json(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// `re.search(r"([+\-]?\d{1,3}(?:\.\d+)?)\s*%", s)` — first signed percentage.
fn first_percent(s: &str) -> Option<f64> {
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_digit()
            || ((c == '+' || c == '-') && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit())
        {
            let start = i;
            if c == '+' || c == '-' {
                i += 1;
            }
            let mut digits = 0;
            while i < bytes.len() && bytes[i].is_ascii_digit() && digits < 3 {
                i += 1;
                digits += 1;
            }
            if i < bytes.len() && bytes[i] == '.' {
                let dot = i;
                let mut j = i + 1;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if j > dot + 1 {
                    i = j;
                }
            }
            let mut k = i;
            while k < bytes.len() && bytes[k].is_whitespace() {
                k += 1;
            }
            if k < bytes.len() && bytes[k] == '%' {
                let token: String = bytes[start..i].iter().collect();
                return token.parse::<f64>().ok();
            }
            i = start + 1;
        } else {
            i += 1;
        }
    }
    None
}

/// `re.search(r"([\d\.]+)", s)` — first run of digits and dots.
fn first_decimal(s: &str) -> Option<f64> {
    let mut run = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() || c == '.' {
            run.push(c);
        } else if !run.is_empty() {
            break;
        }
    }
    if run.is_empty() {
        None
    } else {
        run.parse().ok()
    }
}
