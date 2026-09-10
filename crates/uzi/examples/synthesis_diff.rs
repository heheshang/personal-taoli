//! Differential check: Rust synthesis vs the Python `synthesis.json`.
//!
//! Usage: `cargo run -p personal-taoli-uzi --example synthesis_diff -- <ref_dir>`
//!
//! `is_quant_factor_style` is a real value taken from the reference run: in the
//! original it comes from `detect_quant_signal`, which fetches fund holdings
//! over the network and therefore belongs to data collection, not analysis.
//!
//! Nothing else is excluded: with no agent overrides, synthesis is deterministic.

use serde_json::Value;

fn main() -> anyhow::Result<()> {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| panic!("usage: synthesis_diff <ref_dir>"));
    let raw: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{dir}/raw_data.json"))?)?;
    let dims: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{dir}/dimensions.json"))?)?;
    let panel: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{dir}/panel.json"))?)?;
    let expected: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{dir}/synthesis.json"))?)?;

    let data = personal_taoli_uzi::panel_data()?;
    let features = personal_taoli_uzi::features::extract_features(&raw, &data.feature_tables);

    let comment = |inv: &str, signal: &str, _ctx: &Value| -> String {
        data.personas
            .get(inv)
            .and_then(|m| m.get(signal))
            .or_else(|| data.persona_fallback.get(signal))
            .and_then(|lines| lines.first().cloned())
            .unwrap_or_default()
    };

    let actual = personal_taoli_uzi::synthesis::generate_synthesis(
        &data,
        &raw,
        &dims,
        &panel,
        &features,
        &personal_taoli_uzi::synthesis::SynthesisInputs {
            is_quant_factor_style: true,
            persona_comment: &comment,
            agent_analysis: &Value::Null,
            // Merged by the original's orchestrator from `_data_gaps.json`.
            data_gaps: &serde_json::from_str::<Value>(&std::fs::read_to_string(format!(
                "{dir}/_data_gaps.json"
            ))?)?,
        },
    );

    let mut diffs = Vec::new();
    compare("$", &expected, &actual, &mut diffs);

    println!(
        "rust: overall={} verdict={} style={}",
        actual
            .get("overall_score")
            .map(|v| v.to_string())
            .unwrap_or_default(),
        actual
            .get("verdict_label")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        actual
            .get("detected_style")
            .and_then(|v| v.as_str())
            .unwrap_or("")
    );
    println!(
        "ref : overall={} verdict={} style={}",
        expected
            .get("overall_score")
            .map(|v| v.to_string())
            .unwrap_or_default(),
        expected
            .get("verdict_label")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        expected
            .get("detected_style")
            .and_then(|v| v.as_str())
            .unwrap_or("")
    );
    println!();

    if diffs.is_empty() {
        println!("IDENTICAL — synthesis matched on every field");
        return Ok(());
    }
    let mut by_field: std::collections::BTreeMap<String, usize> = Default::default();
    for d in &diffs {
        let field: String = d.chars().take_while(|c| *c != ':').collect();
        *by_field.entry(field).or_insert(0) += 1;
    }
    println!(
        "{} DIFFERENCE(S) across {} field(s):",
        diffs.len(),
        by_field.len()
    );
    for (f, n) in by_field.iter().take(25) {
        println!("  {n:5} x {f}");
    }
    println!("\nfirst 25:");
    for d in diffs.iter().take(25) {
        println!("  {}", d.chars().take(220).collect::<String>());
    }
    anyhow::bail!("{} difference(s)", diffs.len());
}

fn compare(path: &str, expected: &Value, actual: &Value, diffs: &mut Vec<String>) {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (k, ev) in e {
                match a.get(k) {
                    Some(av) => compare(&format!("{path}.{k}"), ev, av, diffs),
                    None => diffs.push(format!("{path}.{k}: missing in Rust")),
                }
            }
            for k in a.keys() {
                if !e.contains_key(k) {
                    diffs.push(format!("{path}.{k}: extra in Rust"));
                }
            }
        }
        (Value::Array(e), Value::Array(a)) => {
            if e.len() != a.len() {
                diffs.push(format!("{path}: length {} vs {}", e.len(), a.len()));
            }
            for (i, (ev, av)) in e.iter().zip(a.iter()).enumerate() {
                compare(&format!("{path}[{i}]"), ev, av, diffs);
            }
        }
        (Value::Number(x), Value::Number(y)) => {
            let (a, b) = (
                x.as_f64().unwrap_or(f64::NAN),
                y.as_f64().unwrap_or(f64::NAN),
            );
            if !(a == b || (a - b).abs() <= 1e-6 * a.abs().max(1.0)) {
                diffs.push(format!("{path}: expected {x}, got {y}"));
            }
        }
        _ => {
            if expected != actual {
                diffs.push(format!("{path}: expected {expected}, got {actual}"));
            }
        }
    }
}
