//! Interpreter for the compiled UZI rule specs.
//!
//! The 180 investor rules are Python lambdas in the original, which cannot be
//! serialized. `tools/compile_rules.py` derives a declarative expression tree
//! from each lambda's AST and verifies the spec against the original on 301
//! feature vectors; this module evaluates those trees.
//!
//! Semantics deliberately mirror CPython, because the reference artifacts were
//! produced by it:
//!
//! * `and` / `or` return the *operand*, not a boolean (`25 or 0` is `25`), so
//!   `(a or b) > 20` must compare `25` rather than `true`.
//! * a rule whose helper raises is *skipped*, not failed — that distinction is
//!   load-bearing for `fcf_positive`, which scores as neither pass nor fail
//!   when cash-flow direction is unknown.

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

use crate::py;

/// One compiled investor rule.
#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub weight: i64,
    pub spec: Value,
    #[serde(default)]
    pub pass_msg: String,
    #[serde(default)]
    pub fail_msg: String,
}

/// `rules.json` as emitted by the compiler.
#[derive(Debug, Clone, Deserialize)]
pub struct RuleSet {
    pub schema: i64,
    pub groups: BTreeMap<String, Vec<Rule>>,
}

impl RuleSet {
    /// Parses the embedded rule set.
    ///
    /// Fails loudly on a malformed document: a partially-read rule set would
    /// silently change every investor's score.
    pub fn parse(json: &str) -> anyhow::Result<Self> {
        let set: RuleSet = serde_json::from_str(json)
            .map_err(|e| anyhow::anyhow!("embedded rules.json is invalid: {e}"))?;
        if set.schema != crate::SPEC_VERSION {
            anyhow::bail!(
                "embedded rules.json has schema {} but this build expects {}; \
                 re-run tools/compile_rules.py",
                set.schema,
                crate::SPEC_VERSION
            );
        }
        Ok(set)
    }

    pub fn rules_for(&self, group: &str) -> &[Rule] {
        self.groups
            .get(&group.to_ascii_lowercase())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}

/// A rule whose evaluation raised — the caller skips it entirely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleRaised;

/// Python truthiness (`if x:`).
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|x| x != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// `_peg(f)`: PE / growth, or 999 when either side is unusable.
fn peg(f: &Value) -> f64 {
    let pe = py::f(py::get(f, "pe"), 0.0);
    let pe = if pe == 0.0 { 0.0 } else { pe };
    let growth = py::f(py::get(f, "revenue_growth_latest"), 0.0);
    let growth = if growth == 0.0 { 0.0 } else { growth };
    if pe <= 0.0 || growth <= 0.0 {
        999.0
    } else {
        pe / growth
    }
}

/// `_known_fcf(f)`: FCF direction, or raise so the rule is skipped.
fn known_fcf(f: &Value) -> Result<Value, RuleRaised> {
    if !truthy(py::get(f, "fcf_known")) {
        return Err(RuleRaised);
    }
    Ok(Value::Bool(truthy(py::get(f, "fcf_positive"))))
}

/// Evaluates a compiled spec against a feature vector.
pub fn eval(spec: &Value, f: &Value) -> Result<Value, RuleRaised> {
    let obj = spec.as_object().ok_or(RuleRaised)?;

    if let Some(lit) = obj.get("lit") {
        return Ok(lit.clone());
    }

    if let Some(fld) = obj.get("fld") {
        let key = fld.as_str().ok_or(RuleRaised)?;
        // `dict.get(key, default)` substitutes the default only when the key is
        // ABSENT. An explicit null stays null — conflating the two would turn
        // `f.get("x", 0)` into `f.get("x") or 0` and change rule outcomes.
        return Ok(match f.get(key) {
            Some(found) => found.clone(),
            None => obj.get("dflt").cloned().unwrap_or(Value::Null),
        });
    }

    if let Some(tup) = obj.get("tup") {
        let items = tup.as_array().ok_or(RuleRaised)?;
        let mut out = Vec::with_capacity(items.len());
        for i in items {
            out.push(eval(i, f)?);
        }
        return Ok(Value::Array(out));
    }

    if let Some(and) = obj.get("and") {
        let mut result = Value::Null;
        for x in and.as_array().ok_or(RuleRaised)? {
            result = eval(x, f)?;
            if !truthy(&result) {
                return Ok(result);
            }
        }
        return Ok(result);
    }

    if let Some(or) = obj.get("or") {
        let mut result = Value::Null;
        for x in or.as_array().ok_or(RuleRaised)? {
            result = eval(x, f)?;
            if truthy(&result) {
                return Ok(result);
            }
        }
        return Ok(result);
    }

    if let Some(x) = obj.get("not") {
        return Ok(Value::Bool(!truthy(&eval(x, f)?)));
    }

    if let Some(x) = obj.get("neg") {
        let v = eval(x, f)?;
        return Ok(num(-as_num(&v).ok_or(RuleRaised)?));
    }

    if let Some(bin) = obj.get("bin") {
        let op = bin.get("op").and_then(|v| v.as_str()).ok_or(RuleRaised)?;
        let l = eval(bin.get("l").ok_or(RuleRaised)?, f)?;
        let r = eval(bin.get("r").ok_or(RuleRaised)?, f)?;
        let (a, b) = (as_num(&l), as_num(&r));
        // String concatenation, then numeric arithmetic, then the evaluator's
        // "skip" for genuinely incompatible operands.
        if op == "add"
            && let (Value::String(s1), Value::String(s2)) = (&l, &r)
        {
            return Ok(Value::String(format!("{s1}{s2}")));
        }
        let (Some(a), Some(b)) = (a, b) else {
            return Err(RuleRaised);
        };
        return Ok(match op {
            "add" => num(a + b),
            "sub" => num(a - b),
            "mul" => num(a * b),
            "div" => {
                if b == 0.0 {
                    return Err(RuleRaised);
                }
                num(a / b)
            }
            _ => return Err(RuleRaised),
        });
    }

    if let Some(cmp) = obj.get("cmp") {
        let op = cmp.get("op").and_then(|v| v.as_str()).ok_or(RuleRaised)?;
        let l = eval(cmp.get("l").ok_or(RuleRaised)?, f)?;
        let r = eval(cmp.get("r").ok_or(RuleRaised)?, f)?;
        return Ok(Value::Bool(compare(op, &l, &r)?));
    }

    if let Some(branch) = obj.get("if") {
        let c = eval(branch.get("c").ok_or(RuleRaised)?, f)?;
        return if truthy(&c) {
            eval(branch.get("t").ok_or(RuleRaised)?, f)
        } else {
            eval(branch.get("e").ok_or(RuleRaised)?, f)
        };
    }

    if let Some(any_in) = obj.get("any_in") {
        if let Some(guard) = any_in.get("guard").filter(|g| !g.is_null())
            && !truthy(&eval(guard, f)?)
        {
            return Ok(Value::Bool(false));
        }
        let hay = eval(any_in.get("hay").ok_or(RuleRaised)?, f)?;
        let needles = any_in
            .get("needles")
            .and_then(|v| v.as_array())
            .ok_or(RuleRaised)?;
        return Ok(Value::Bool(needles.iter().any(|n| contains(&hay, n))));
    }

    if obj.contains_key("peg") {
        return Ok(num(peg(f)));
    }

    if obj.contains_key("fcf") {
        return known_fcf(f);
    }

    if let Some(name) = obj.get("call").and_then(|v| v.as_str()) {
        let args = obj
            .get("args")
            .and_then(|v| v.as_array())
            .ok_or(RuleRaised)?;
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(eval(a, f)?);
        }
        return call_builtin(name, &vals);
    }

    if let Some(meth) = obj.get("meth").and_then(|v| v.as_str()) {
        let recv = eval(obj.get("recv").ok_or(RuleRaised)?, f)?;
        let s = match recv {
            Value::String(s) => s,
            Value::Null => String::new(),
            other => py::py_str(&other),
        };
        return Ok(Value::String(match meth {
            "lower" => s.to_lowercase(),
            "upper" => s.to_uppercase(),
            "strip" => s.trim().to_string(),
            _ => return Err(RuleRaised),
        }));
    }

    Err(RuleRaised)
}

/// Python-style `==` over JSON values: numbers compare by value, not encoding.
fn json_eq(l: &Value, r: &Value) -> bool {
    match (l, r) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => x == y,
            _ => a == b,
        },
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| json_eq(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).map(|w| json_eq(v, w)).unwrap_or(false))
        }
        _ => l == r,
    }
}

fn contains(hay: &Value, needle: &Value) -> bool {
    match (hay, needle) {
        (Value::String(h), Value::String(n)) => h.contains(n.as_str()),
        (Value::Array(items), n) => items.iter().any(|i| json_eq(i, n)),
        _ => false,
    }
}

fn as_num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Serialises a computed double the way `serde_json` would for the same value,
/// keeping whole results integral (`25` rather than `25.0`) so downstream
/// label formatting matches Python's `str()`.
fn num(x: f64) -> Value {
    if x.fract() == 0.0 && x.abs() < 9.007_199_254_740_992e15 {
        Value::Number(serde_json::Number::from(x as i64))
    } else {
        serde_json::Number::from_f64(x)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }
}

/// Ordering comparisons raise in Python when the operands do not support it —
/// e.g. `None >= 24`. That raise is meaningful: the evaluator *skips* the rule
/// rather than failing it, so reporting `false` here would silently convert a
/// skip into a negative verdict and move the investor's score.
fn compare(op: &str, l: &Value, r: &Value) -> Result<bool, RuleRaised> {
    // Equality is numeric across JSON number representations: Python's `2.0 == 2`
    // is true, whereas comparing `serde_json::Number` directly would distinguish
    // the float and integer encodings.
    match op {
        "eq" => return Ok(json_eq(l, r)),
        "ne" => return Ok(!json_eq(l, r)),
        "is" => return Ok(json_eq(l, r)),
        "isnot" => return Ok(!json_eq(l, r)),
        "in" => return Ok(contains(r, l)),
        "notin" => return Ok(!contains(r, l)),
        _ => {}
    }
    if let (Value::String(a), Value::String(b)) = (l, r) {
        return Ok(match op {
            "gt" => a > b,
            "lt" => a < b,
            "ge" => a >= b,
            "le" => a <= b,
            _ => return Err(RuleRaised),
        });
    }
    let (Some(a), Some(b)) = (as_num(l), as_num(r)) else {
        return Err(RuleRaised);
    };
    Ok(match op {
        "gt" => a > b,
        "lt" => a < b,
        "ge" => a >= b,
        "le" => a <= b,
        _ => return Err(RuleRaised),
    })
}

fn call_builtin(name: &str, args: &[Value]) -> Result<Value, RuleRaised> {
    match name {
        "abs" => Ok(num(as_num(args.first().ok_or(RuleRaised)?)
            .ok_or(RuleRaised)?
            .abs())),
        "max" => {
            let mut best = f64::NEG_INFINITY;
            for a in args {
                best = best.max(as_num(a).ok_or(RuleRaised)?);
            }
            Ok(num(best))
        }
        "min" => {
            let mut best = f64::INFINITY;
            for a in args {
                best = best.min(as_num(a).ok_or(RuleRaised)?);
            }
            Ok(num(best))
        }
        "str" => Ok(Value::String(
            args.first().map(py::py_str).unwrap_or_default(),
        )),
        "bool" => Ok(Value::Bool(args.first().map(truthy).unwrap_or(false))),
        "len" => Ok(Value::Number(serde_json::Number::from(
            args.first()
                .map(|v| match v {
                    Value::String(s) => s.chars().count(),
                    Value::Array(a) => a.len(),
                    Value::Object(o) => o.len(),
                    _ => 0,
                })
                .unwrap_or(0) as i64,
        ))),
        "int" => Ok(num(as_num(args.first().ok_or(RuleRaised)?)
            .ok_or(RuleRaised)?
            .floor())),
        "float" => Ok(num(
            as_num(args.first().ok_or(RuleRaised)?).ok_or(RuleRaised)?
        )),
        "round" => {
            let x = as_num(args.first().ok_or(RuleRaised)?).ok_or(RuleRaised)?;
            let digits = args
                .get(1)
                .and_then(as_num)
                .map(|d| d.max(0.0) as usize)
                .unwrap_or(0);
            Ok(num(py::round_to(x, digits)))
        }
        _ => Err(RuleRaised),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn compile(spec: Value) -> Value {
        spec
    }

    #[test]
    fn or_returns_the_operand_not_a_bool() {
        // `(f.get("a") or f.get("b", 0)) > 20` must compare the operand.
        let spec = compile(json!({"cmp": {"op": "gt", "l": {"or": [
            {"fld": "a", "dflt": null}, {"fld": "b", "dflt": 0}
        ]}, "r": {"lit": 20}}}));
        assert_eq!(eval(&spec, &json!({"a": 25, "b": 0})).unwrap(), json!(true));
        // `5 or 30` is 5 in Python, so the comparison is `5 > 20`.
        assert_eq!(
            eval(&spec, &json!({"a": 5, "b": 30})).unwrap(),
            json!(false)
        );
        // `0 or 30` is 30, so the fallback only engages on a falsy left side.
        assert_eq!(eval(&spec, &json!({"a": 0, "b": 30})).unwrap(), json!(true));
        assert_eq!(eval(&spec, &json!({"a": 0, "b": 3})).unwrap(), json!(false));
    }

    /// `dict.get(key, default)` ignores the default when the key is present,
    /// even if the stored value is null.
    #[test]
    fn field_default_applies_only_when_the_key_is_absent() {
        let spec = json!({"fld": "x", "dflt": 0});
        assert_eq!(eval(&spec, &json!({})).unwrap(), json!(0));
        assert_eq!(eval(&spec, &json!({"x": null})).unwrap(), Value::Null);
        assert_eq!(eval(&spec, &json!({"x": 7})).unwrap(), json!(7));
    }

    #[test]
    fn and_short_circuits_with_the_operand() {
        let spec = json!({"and": [{"fld": "a", "dflt": 0}, {"cmp": {"op": "gt", "l": {"fld": "a", "dflt": 0}, "r": {"lit": 5}}}]});
        assert_eq!(eval(&spec, &json!({"a": 0})).unwrap(), json!(0));
        assert_eq!(eval(&spec, &json!({"a": 10})).unwrap(), json!(true));
    }

    #[test]
    fn fcf_helper_skips_when_direction_is_unknown() {
        let spec = json!({"fcf": true});
        assert_eq!(
            eval(&spec, &json!({"fcf_known": true, "fcf_positive": true})).unwrap(),
            json!(true)
        );
        assert_eq!(
            eval(&spec, &json!({"fcf_known": false})).unwrap_err(),
            RuleRaised
        );
    }

    #[test]
    fn peg_is_999_when_inputs_are_unusable() {
        assert_eq!(peg(&json!({"pe": 20, "revenue_growth_latest": 10})), 2.0);
        assert_eq!(peg(&json!({"pe": 0, "revenue_growth_latest": 10})), 999.0);
        assert_eq!(peg(&json!({"pe": 20, "revenue_growth_latest": 0})), 999.0);
    }

    #[test]
    fn any_in_tests_substrings_of_both_operands() {
        let spec = json!({"any_in": {
            "hay": {"bin": {"op": "add", "l": {"fld": "industry", "dflt": ""}, "r": {"fld": "name", "dflt": ""}}},
            "needles": ["AI", "云"], "guard": null
        }});
        assert_eq!(
            eval(&spec, &json!({"industry": "白酒", "name": "某AI公司"})).unwrap(),
            json!(true)
        );
        assert_eq!(
            eval(&spec, &json!({"industry": "白酒", "name": "茅台"})).unwrap(),
            json!(false)
        );
    }

    #[test]
    fn string_concatenation_in_add() {
        let spec = json!({"bin": {"op": "add", "l": {"fld": "a", "dflt": ""}, "r": {"fld": "b", "dflt": ""}}});
        assert_eq!(
            eval(&spec, &json!({"a": "x", "b": "y"})).unwrap(),
            json!("xy")
        );
    }

    #[test]
    fn embedded_ruleset_parses() {
        let set = RuleSet::parse(crate::RULES_JSON).expect("embedded rules.json");
        assert_eq!(set.groups.len(), 43);
        assert_eq!(set.groups.values().map(|v| v.len()).sum::<usize>(), 180);
    }
}
