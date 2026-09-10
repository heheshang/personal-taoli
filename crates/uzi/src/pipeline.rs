//! Analysis pipeline: `raw_data.json` → the three downstream artifacts.
//!
//! This is the seam between Python data collection and Python rendering. The
//! upstream project computes scoring, the investor panel and synthesis in
//! Python and writes:
//!
//! ```text
//! .cache/<ticker>/raw_data.json      ← Python (collection + institutional modeling)
//! .cache/<ticker>/dimensions.json    ← this module
//! .cache/<ticker>/panel.json         ← this module
//! .cache/<ticker>/synthesis.json     ← this module
//! ```
//!
//! The upstream renderer (`assemble_report.assemble`) reads those same three
//! files, so writing them here substitutes the Rust analysis for the Python one
//! without touching the renderer.
//!
//! Two inputs cannot be computed from `raw_data.json` alone and are passed in:
//! the quant-factor flag (fund holdings, fetched over the network) and the
//! data-gap list (produced by the collection stage).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::panel_data::PanelData;

/// What the pipeline produced, for the caller to report.
#[derive(Debug)]
pub struct AnalyzeOutcome {
    pub cache_dir: PathBuf,
    pub ticker: String,
    pub overall_score: f64,
    pub verdict_label: String,
    pub detected_style: String,
    pub panel_consensus: f64,
    pub investor_count: usize,
    pub dimensions_path: PathBuf,
    pub panel_path: PathBuf,
    pub synthesis_path: PathBuf,
}

/// Persona comment for one investor, with `{name}`-style variables substituted.
///
/// The original uses an **unseeded** `random.choice`, so its output differs on
/// every run and cannot be reproduced. A stable index derived from the
/// (investor, signal) pair is used instead: same input, same report. The field
/// is decorative narration, not analysis.
///
/// Substitution mirrors `get_comment`: the seven known keys come from `ctx`, and
/// a template referencing anything else is returned verbatim (the original
/// catches `KeyError`/`IndexError` and does the same).
pub fn persona_comment(data: &PanelData, investor: &str, signal: &str, ctx: &Value) -> String {
    let lines = data
        .personas
        .get(investor)
        .and_then(|by_signal| by_signal.get(signal))
        .or_else(|| data.persona_fallback.get(signal));
    let Some(lines) = lines else {
        return String::new();
    };
    if lines.is_empty() {
        return String::new();
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in investor
        .bytes()
        .chain(std::iter::once(0))
        .chain(signal.bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let line = &lines[(hash % lines.len() as u64) as usize];
    substitute(line, ctx)
}

/// `line.format(**ctx)` over the seven context keys, with the original's
/// fall-back to the unformatted line when a placeholder is unknown.
fn substitute(line: &str, ctx: &Value) -> String {
    const KEYS: [&str; 7] = ["roe", "pe", "price", "name", "industry", "growth", "stage"];
    let value_of = |key: &str| -> String {
        let v = crate::py::get(ctx, key);
        if v.is_null() {
            "—".to_string()
        } else {
            crate::py::py_str(v)
        }
    };

    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' if i + 1 < chars.len() && chars[i + 1] == '{' => {
                out.push('{');
                i += 2;
            }
            '}' if i + 1 < chars.len() && chars[i + 1] == '}' => {
                out.push('}');
                i += 2;
            }
            '{' => {
                let Some(close) = (i + 1..chars.len()).find(|&j| chars[j] == '}') else {
                    out.push(chars[i]);
                    i += 1;
                    continue;
                };
                let name: String = chars[i + 1..close].iter().collect();
                if !KEYS.contains(&name.as_str()) {
                    // Unknown placeholder: Python's `format` raises and the
                    // caller returns the raw line.
                    return line.to_string();
                }
                out.push_str(&value_of(&name));
                i = close + 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Runs the analysis for one cached ticker and writes the three artifacts.
///
/// `cache_dir` is `.cache/<full ticker>` as the upstream layout defines it.
pub fn analyze(
    cache_dir: &Path,
    is_quant_factor_style: bool,
    locked_school: Option<&str>,
) -> Result<AnalyzeOutcome> {
    let raw: Value = read_json(&cache_dir.join("raw_data.json"))
        .context("raw_data.json is required; run the fetch stage first")?;
    let data = crate::panel_data()?;
    let rules = crate::rules()?;

    let ticker = raw
        .get("ticker")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if ticker.is_empty() {
        anyhow::bail!("raw_data.json has no `ticker`");
    }

    // 1 · features → 2 · dimensions (the 19 scored dimensions)
    let features = crate::features::extract_features(&raw, &data.feature_tables);
    let dimensions = crate::score::score_dimensions(&raw)?;

    // 3 · the investor panel
    let panel = crate::panel::generate_panel(
        &data,
        &rules,
        &raw,
        &features,
        locked_school,
        |investor, signal, ctx| persona_comment(&data, investor, signal, ctx),
    );

    // 4 · synthesis. `_data_gaps.json` is optional; the collection stage writes
    //     it only when it recorded gaps.
    let agent_analysis = Value::Null;
    let data_gaps = read_json(&cache_dir.join("_data_gaps.json")).unwrap_or(Value::Null);
    let synthesis = crate::synthesis::generate_synthesis(
        &data,
        &raw,
        &dimensions,
        &panel,
        &features,
        &crate::synthesis::SynthesisInputs {
            is_quant_factor_style,
            persona_comment: &|investor, signal, ctx| persona_comment(&data, investor, signal, ctx),
            agent_analysis: &agent_analysis,
            data_gaps: &data_gaps,
        },
    );

    let dimensions_path = write_json(&cache_dir.join("dimensions.json"), &dimensions)?;
    let panel_path = write_json(&cache_dir.join("panel.json"), &panel)?;
    let synthesis_path = write_json(&cache_dir.join("synthesis.json"), &synthesis)?;

    Ok(AnalyzeOutcome {
        cache_dir: cache_dir.to_path_buf(),
        ticker,
        overall_score: synthesis
            .get("overall_score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        verdict_label: synthesis
            .get("verdict_label")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        detected_style: synthesis
            .get("detected_style")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        panel_consensus: synthesis
            .get("panel_consensus")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        investor_count: panel
            .get("investors")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0),
        dimensions_path,
        panel_path,
        synthesis_path,
    })
}

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Writes with the upstream's formatting (`ensure_ascii=False, indent=2`) so a
/// diff against a Python-produced artifact shows only real differences.
fn write_json(path: &Path, value: &Value) -> Result<PathBuf> {
    let text = serde_json::to_string_pretty(value)
        .with_context(|| format!("failed to serialize {}", path.display()))?;
    std::fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persona_choice_is_stable_for_the_same_inputs() {
        let data = crate::panel_data().unwrap();
        let ctx = serde_json::json!({"name": "贵州茅台", "industry": "白酒"});
        let a = persona_comment(&data, "buffett", "bullish", &ctx);
        let b = persona_comment(&data, "buffett", "bullish", &ctx);
        assert_eq!(a, b);
        assert!(!a.is_empty());
        // Different signals draw from different pools.
        let bearish = persona_comment(&data, "buffett", "bearish", &ctx);
        assert_ne!(a, bearish);
    }

    #[test]
    fn persona_templates_have_their_variables_substituted() {
        let ctx = serde_json::json!({"name": "贵州茅台", "industry": "白酒", "roe": "32.5"});
        assert_eq!(
            substitute("{name}：资产真实、现金流真实。", &ctx),
            "贵州茅台：资产真实、现金流真实。"
        );
        assert_eq!(
            substitute("故事在 {industry} 里十个死九个", &ctx),
            "故事在 白酒 里十个死九个"
        );
        assert_eq!(substitute("ROE {roe}% 稳", &ctx), "ROE 32.5% 稳");
        // A missing context key falls back to the em dash, as Python's
        // `ctx.get(k, "-")` does.
        assert_eq!(substitute("{pe}", &ctx), "—");
        // An unknown placeholder returns the line untouched (Python catches the
        // KeyError from `str.format` and does the same).
        let raw = "百分号 {unknown_key} 保留";
        assert_eq!(substitute(raw, &ctx), raw);
        // Literal braces survive.
        assert_eq!(substitute("{{literal}}", &ctx), "{literal}");
    }

    #[test]
    fn persona_choice_falls_back_for_unknown_investors() {
        let data = crate::panel_data().unwrap();
        let line = persona_comment(&data, "not_an_investor", "neutral", &Value::Null);
        assert!(!line.is_empty());
    }

    #[test]
    fn missing_raw_data_is_an_error_not_a_panic() {
        let dir = std::env::temp_dir().join("uzi-pipeline-missing");
        let _ = std::fs::create_dir_all(&dir);
        let err = analyze(&dir, false, None).unwrap_err();
        assert!(format!("{err:#}").contains("raw_data.json"));
    }
}
