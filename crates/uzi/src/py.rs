//! Python-semantics helpers.
//!
//! The UZI scoring core is being ported from Python, and the reference values
//! it must reproduce are Python-derived. Rather than "improve" semantics and
//! then chase diffs, these helpers mirror CPython's observable behaviour for
//! the narrow operations the scoring functions actually use.

use serde_json::{Map, Value};

/// Mirrors Python's `_f(v, default=0.0)`:
/// `float(str(v).replace("%","").replace(",","").replace("+",""))`.
///
/// Returns `default` on any parse failure, which is how the Python swallows
/// `None`, empty strings and non-numeric text.
pub fn f(v: &Value, default: f64) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(default),
        Value::String(s) => {
            // Python's `str.replace` removes every occurrence, in order:
            // "%", then ",", then "+".
            let cleaned: String = s
                .chars()
                .filter(|c| *c != '%' && *c != ',' && *c != '+')
                .collect();
            // Python's `float()` tolerates surrounding whitespace.
            cleaned.trim().parse::<f64>().unwrap_or(default)
        }
        _ => default,
    }
}

/// Mirrors Python's `str()` for the JSON value kinds that reach a label.
///
/// Lists and dicts use their Python `repr`, which is only indistinguishable
/// from JSON for the scalar cases the callers pass; strings are unquoted,
/// numbers keep Python's shortest-roundtrip spelling.
pub fn py_str(v: &Value) -> String {
    match v {
        Value::Null => "None".to_string(),
        Value::Bool(b) => if *b { "True" } else { "False" }.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `d.get(key)` — a missing key and an explicit `null` both yield `Null`.
pub fn get<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.get(key).unwrap_or(&Value::Null)
}

/// Mirrors Python's `d.get(key, default)`.
pub fn get_or<'a>(v: &'a Value, key: &str, default: &'a Value) -> &'a Value {
    match v.get(key) {
        Some(found) => found,
        None => default,
    }
}

/// Mirrors `(dims.get(key) or {}).get("data") or {}`.
///
/// The doubled `or {}` means an empty object collapses to `Null`, which is
/// indistinguishable from a missing dimension to every reader below.
pub fn data<'a>(raw: &'a Value, dim_key: &str) -> &'a Value {
    let dim = get(get(raw, "dimensions"), dim_key);
    let payload = get(dim, "data");
    if payload.is_null() {
        &Value::Null
    } else {
        payload
    }
}

/// `v or []` for arrays.
pub fn arr(v: &Value) -> &[Value] {
    v.as_array().map(|a| a.as_slice()).unwrap_or(&[])
}

/// `v or {}` for objects.
pub fn obj(v: &Value) -> Option<&Map<String, Value>> {
    v.as_object()
}

/// Integer reads the scoring functions perform on counts.
///
/// Python integers are unbounded and `//` floors; JSON may carry either form,
/// so a float is floored here the way `int`/`//` would land for the
/// non-negative counts these fields hold.
pub fn int(v: &Value, default: i64) -> i64 {
    match v {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i
            } else if let Some(u) = n.as_u64() {
                u.min(i64::MAX as u64) as i64
            } else {
                n.as_f64().map(|x| x.floor() as i64).unwrap_or(default)
            }
        }
        Value::String(s) => s.trim().parse::<i64>().unwrap_or(default),
        _ => default,
    }
}

/// Mirrors `int((a or {}).get(k) or 0)`.
pub fn int_or_zero(v: &Value) -> i64 {
    if v.is_null() { 0 } else { int(v, 0) }
}

/// `re.search(r'(\d+)', s)` — first run of ASCII digits.
pub fn first_digits(s: &str) -> Option<i64> {
    let mut run = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            run.push(c);
        } else if !run.is_empty() {
            break;
        }
    }
    if run.is_empty() {
        None
    } else {
        run.parse::<i64>().ok()
    }
}

/// Python's `round(x, n)` for the one-decimal scores.
///
/// Routed through Rust's formatter so the result is the double nearest the
/// decimal string Python's `round` would emit, including half-to-even ties.
pub fn round_to(x: f64, digits: usize) -> f64 {
    format!("{:.*}", digits, x).parse::<f64>().unwrap_or(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn f_matches_python_string_cleaning() {
        // Percent, thousands separator and leading plus are stripped.
        assert_eq!(f(&json!("32.5%"), 0.0), 32.5);
        assert_eq!(f(&json!("+1.5%"), 0.0), 1.5);
        assert_eq!(f(&json!("1,234.5"), 0.0), 1234.5);
        assert_eq!(f(&json!(15.1931), 0.0), 15.1931);
        // Everything Python's float() rejects falls back.
        assert_eq!(f(&json!(null), 7.0), 7.0);
        assert_eq!(f(&json!(""), 7.0), 7.0);
        assert_eq!(f(&json!("—"), 7.0), 7.0);
        assert_eq!(f(&json!(true), 7.0), 7.0);
    }

    #[test]
    fn py_str_matches_python_str() {
        assert_eq!(py_str(&json!(null)), "None");
        assert_eq!(py_str(&json!(true)), "True");
        assert_eq!(py_str(&json!("18.04")), "18.04");
        assert_eq!(py_str(&json!(18.04)), "18.04");
    }

    #[test]
    fn first_digits_takes_the_leading_run() {
        assert_eq!(first_digits("50"), Some(50));
        assert_eq!(first_digits("50 分位"), Some(50));
        assert_eq!(first_digits("PE 18.04"), Some(18));
        assert_eq!(first_digits("分位"), None);
        assert_eq!(first_digits(""), None);
    }

    #[test]
    fn round_to_uses_half_to_even_like_python() {
        assert_eq!(round_to(58.732_394, 1), 58.7);
        assert_eq!(round_to(0.25, 1), 0.2);
        assert_eq!(round_to(0.35, 1), 0.3);
    }

    #[test]
    fn data_treats_missing_and_empty_alike() {
        let raw = json!({
            "dimensions": {
                "present": { "data": { "roe": "32.5%" } },
                "empty": { "data": {} },
                "no_data_key": {}
            }
        });
        assert_eq!(get(data(&raw, "present"), "roe"), &json!("32.5%"));
        assert!(data(&raw, "empty").is_object());
        assert_eq!(data(&raw, "no_data_key"), &Value::Null);
        assert_eq!(data(&raw, "absent"), &Value::Null);
    }
}
