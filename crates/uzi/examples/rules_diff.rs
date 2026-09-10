//! Differential check: Rust rule interpreter vs the original Python lambdas.
//!
//! Usage: `cargo run -p personal-taoli-uzi --example rules_diff -- <vectors.json>`
//!
//! `tools/compile_rules.py` records, for every vector and every rule, whether
//! the *original lambda* returned a truthy value or raised. This replays those
//! vectors through the Rust interpreter and reports any disagreement, which is
//! what catches interpreter drift the in-Python checks cannot see.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use personal_taoli_uzi::rules::{RuleSet, eval, truthy};

#[derive(Deserialize)]
struct Expected {
    ok: bool,
    val: Option<bool>,
}

#[derive(Deserialize)]
struct Vector {
    features: Value,
    expected: BTreeMap<String, Expected>,
}

#[derive(Deserialize)]
struct Doc {
    schema: i64,
    vectors: Vec<Vector>,
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| panic!("usage: rules_diff <rule_vectors.json>"));
    let doc: Doc = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    if doc.schema != personal_taoli_uzi::SPEC_VERSION {
        anyhow::bail!("vector file schema {} is stale", doc.schema);
    }

    let set = RuleSet::parse(personal_taoli_uzi::RULES_JSON)?;
    let mut index: BTreeMap<String, (&str, &personal_taoli_uzi::Rule)> = BTreeMap::new();
    for (group, rules) in &set.groups {
        for r in rules {
            index.insert(format!("{group}.{}", r.id), (group, r));
        }
    }

    let mut diffs: Vec<String> = Vec::new();
    let mut compared = 0usize;

    for (vi, vector) in doc.vectors.iter().enumerate() {
        for (key, want) in &vector.expected {
            let Some((_group, rule)) = index.get(key) else {
                diffs.push(format!("{key}: not present in compiled rule set"));
                continue;
            };
            let got = eval(&rule.spec, &vector.features);
            compared += 1;
            match (&got, want.ok) {
                (Err(_), false) => {}
                (Err(_), true) => diffs.push(format!(
                    "{key}[vec{vi}]: lambda scored {}, Rust raised (skipped)",
                    want.val.unwrap_or(false)
                )),
                (Ok(v), false) => {
                    diffs.push(format!("{key}[vec{vi}]: lambda raised, Rust returned {v}"))
                }
                (Ok(v), true) => {
                    let got_bool = truthy(v);
                    if Some(got_bool) != want.val {
                        diffs.push(format!(
                            "{key}[vec{vi}]: lambda={:?} Rust={:?}",
                            want.val, got_bool
                        ));
                    }
                }
            }
        }
    }

    // Every compiled rule must appear in the vector file; a rule the harness
    // never exercises would be unverified.
    if let Some(first) = doc.vectors.first() {
        for key in index.keys() {
            if !first.expected.contains_key(key) {
                diffs.push(format!("{key}: never exercised by the vector file"));
            }
        }
    }

    println!("rules in compiled set : {}", index.len());
    println!("vectors               : {}", doc.vectors.len());
    println!("rule evaluations      : {compared}");
    println!();
    if diffs.is_empty() {
        println!("IDENTICAL — Rust interpreter matches the original lambdas on every vector");
        return Ok(());
    }
    println!("{} DIFFERENCE(S):", diffs.len());
    for d in diffs.iter().take(25) {
        println!("  {d}");
    }
    anyhow::bail!("{} difference(s)", diffs.len());
}
