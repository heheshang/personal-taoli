//! Differential check: Rust panel vs the Python `panel.json` reference.
//!
//! Usage: `cargo run -p personal-taoli-uzi --example panel_diff -- <ref_dir>`
//!
//! `comment` is excluded deliberately: the original selects it with an
//! *unseeded* `random.choice`, so a stored value is not reproducible. Every
//! other field is compared exactly.

use std::collections::BTreeMap;

use serde_json::Value;

fn main() -> anyhow::Result<()> {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| panic!("usage: panel_diff <ref_dir>"));
    let raw: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{dir}/raw_data.json"))?)?;
    let expected: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{dir}/panel.json"))?)?;

    let data = personal_taoli_uzi::panel_data()?;
    let rules = personal_taoli_uzi::rules()?;
    let features = personal_taoli_uzi::features::extract_features(&raw, &data.feature_tables);

    // Deterministic stand-in for the unseeded persona choice: take the first
    // candidate. Only `comment` depends on it.
    let actual = personal_taoli_uzi::panel::generate_panel(
        &data,
        &rules,
        &raw,
        &features,
        None,
        |inv, signal, _ctx| {
            data.personas
                .get(inv)
                .and_then(|m| m.get(signal))
                .or_else(|| data.persona_fallback.get(signal))
                .and_then(|lines| lines.first().cloned())
                .unwrap_or_default()
        },
    );

    let mut diffs: Vec<String> = Vec::new();
    compare("$", &expected, &actual, &mut diffs);

    println!(
        "rust: consensus={} investors={}",
        actual
            .get("panel_consensus")
            .map(|v| v.to_string())
            .unwrap_or_default(),
        actual
            .get("investors")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    );
    println!(
        "ref : consensus={} investors={}",
        expected
            .get("panel_consensus")
            .map(|v| v.to_string())
            .unwrap_or_default(),
        expected
            .get("investors")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    );
    println!();

    if diffs.is_empty() {
        println!("IDENTICAL — panel matched (comment excluded: unseeded random in the source)");
        return Ok(());
    }
    // Group by field path so one systemic error does not flood the output.
    let mut by_field: BTreeMap<String, usize> = BTreeMap::new();
    for d in &diffs {
        let field = d.split(':').next().unwrap_or(d).to_string();
        *by_field.entry(field).or_insert(0) += 1;
    }
    println!(
        "{} DIFFERENCE(S) across {} field(s):",
        diffs.len(),
        by_field.len()
    );
    for (field, n) in by_field.iter().take(30) {
        println!("  {n:5} x {field}");
    }
    println!("\nfirst 20:");
    for d in diffs.iter().take(20) {
        println!("  {d}");
    }
    anyhow::bail!("{} difference(s)", diffs.len());
}

fn compare(path: &str, expected: &Value, actual: &Value, diffs: &mut Vec<String>) {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (k, ev) in e {
                // The one intentionally non-reproducible field.
                if k == "comment" {
                    continue;
                }
                match a.get(k) {
                    Some(av) => compare(&format!("{path}.{k}"), ev, av, diffs),
                    None => diffs.push(format!("{path}.{k}: missing in Rust")),
                }
            }
            for k in a.keys() {
                if k != "comment" && !e.contains_key(k) {
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
