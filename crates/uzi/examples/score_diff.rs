//! Differential check: Rust `score_dimensions` vs the Python reference.
//!
//! Usage: `cargo run -p personal-taoli-uzi --example score_diff -- <ref_dir>`
//! where `<ref_dir>` holds `raw_data.json` and `dimensions.json` produced by
//! the original Python pipeline.
//!
//! Exits non-zero on the first structural difference, printing its path.

use std::path::PathBuf;

use serde_json::Value;

fn compare(path: &str, expected: &Value, actual: &Value, diffs: &mut Vec<String>) {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (k, ev) in e {
                match a.get(k) {
                    Some(av) => compare(&format!("{path}.{k}"), ev, av, diffs),
                    None => diffs.push(format!("{path}.{k}: missing in Rust output")),
                }
            }
            for k in a.keys() {
                if !e.contains_key(k) {
                    diffs.push(format!("{path}.{k}: extra in Rust output"));
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
        (Value::Number(e), Value::Number(a)) => {
            let (ef, af) = (
                e.as_f64().unwrap_or(f64::NAN),
                a.as_f64().unwrap_or(f64::NAN),
            );
            if !(ef == af || (ef - af).abs() < 1e-9) {
                diffs.push(format!("{path}: expected {e}, got {a}"));
            }
        }
        _ => {
            if expected != actual {
                diffs.push(format!("{path}: expected {expected}, got {actual}"));
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| panic!("usage: score_diff <ref_dir>")),
    );
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("raw_data.json"))?)?;
    let expected: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("dimensions.json"))?)?;

    let actual = personal_taoli_uzi::score_dimensions(&raw)?;

    let mut diffs = Vec::new();
    compare("$", &expected, &actual, &mut diffs);

    let dims = actual["dimensions"]
        .as_object()
        .map(|m| m.len())
        .unwrap_or(0);
    println!(
        "rust: dims={dims} fundamental_score={}",
        actual["fundamental_score"]
    );
    println!(
        "ref : dims={} fundamental_score={}",
        expected["dimensions"]
            .as_object()
            .map(|m| m.len())
            .unwrap_or(0),
        expected["fundamental_score"]
    );
    println!();

    if diffs.is_empty() {
        println!("IDENTICAL — {} dimensions matched", dims);
        return Ok(());
    }
    println!("{} DIFFERENCE(S):", diffs.len());
    for d in &diffs {
        println!("  {d}");
    }
    anyhow::bail!("{} difference(s) vs reference", diffs.len());
}
