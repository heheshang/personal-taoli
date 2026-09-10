#!/usr/bin/env python3
"""Compile UZI-Skill rule predicates into declarative specs, and verify them.

The 180 investor rules are Python lambdas, which cannot be serialized. Rather
than hand-translating each to Rust (180 chances to introduce a typo), this
parses each lambda's AST and emits a small declarative expression tree that a
Rust interpreter evaluates.

Correctness is established by construction plus differential testing:

  1. the spec is derived from the lambda's AST, not retyped;
  2. a Python reference interpreter evaluates the spec;
  3. that result is compared against the *original lambda* on the real feature
     vector and on randomized perturbations.

Any disagreement fails the run, so a silent mistranslation cannot ship.

Outputs (consumed by `include_str!` in the Rust crate):
  out/rules.json     — 180 rule specs, grouped by investor
  out/investors.json — 66 investor records (pure metadata)
"""
from __future__ import annotations

import ast
import inspect
import json
import random
import sys
from pathlib import Path

SCRIPTS = Path("/tmp/uzi-probe/skills/deep-analysis/scripts")
OUT = Path(__file__).resolve().parent / "out"
sys.path.insert(0, str(SCRIPTS))

import lib.investor_criteria as ic  # noqa: E402
import lib.investor_db as idb  # noqa: E402
import lib.stock_features as sf  # noqa: E402

SPEC_VERSION = 1

# ── AST → spec ────────────────────────────────────────────────────────────


class CompileError(Exception):
    pass


# The receiver name differs per rule: lambdas use `f`, the named helpers take
# `features`. Set once per rule before compiling its body.
RECV = "f"


def _const_value(e: dict):
    """Fold a spec node that is a compile-time constant, else (False, None)."""
    if "lit" in e:
        return True, e["lit"]
    if "neg" in e:
        ok, v = _const_value(e["neg"])
        return (True, -v) if ok else (False, None)
    return False, None


def compile_expr(node: ast.AST) -> dict:
    if isinstance(node, ast.Constant):
        return {"lit": node.value}

    if isinstance(node, ast.Name):
        if node.id == "f":
            raise CompileError("bare `f`")
        if node.id in ("True", "False", "None"):
            return {"lit": {"True": True, "False": False, "None": None}[node.id]}
        raise CompileError(f"unexpected name {node.id!r}")

    if isinstance(node, ast.Tuple):
        return {"tup": [compile_expr(e) for e in node.elts]}

    if isinstance(node, ast.List):
        return {"tup": [compile_expr(e) for e in node.elts]}

    if isinstance(node, ast.BinOp):
        op = {ast.Add: "add", ast.Sub: "sub", ast.Div: "div", ast.Mult: "mul"}.get(type(node.op))
        if op is None:
            raise CompileError(f"operator {type(node.op).__name__}")
        return {"bin": {"op": op, "l": compile_expr(node.left), "r": compile_expr(node.right)}}

    if isinstance(node, ast.UnaryOp):
        if isinstance(node.op, ast.Not):
            return {"not": compile_expr(node.operand)}
        if isinstance(node.op, ast.USub):
            return {"neg": compile_expr(node.operand)}
        raise CompileError(f"unary {type(node.op).__name__}")

    if isinstance(node, ast.BoolOp):
        key = "and" if isinstance(node.op, ast.And) else "or"
        return {key: [compile_expr(v) for v in node.values]}

    if isinstance(node, ast.Compare):
        # Chained comparisons are an implicit `and` over adjacent pairs.
        parts = []
        left = node.left
        for op, right in zip(node.ops, node.comparators):
            name = {
                ast.Gt: "gt", ast.Lt: "lt", ast.GtE: "ge", ast.LtE: "le",
                ast.Eq: "eq", ast.NotEq: "ne", ast.In: "in", ast.NotIn: "notin",
                ast.Is: "is", ast.IsNot: "isnot",
            }.get(type(op))
            if name is None:
                raise CompileError(f"comparison {type(op).__name__}")
            parts.append({"cmp": {"op": name, "l": compile_expr(left), "r": compile_expr(right)}})
            left = right
        return parts[0] if len(parts) == 1 else {"and": parts}

    if isinstance(node, ast.IfExp):
        return {"if": {"c": compile_expr(node.test), "t": compile_expr(node.body), "e": compile_expr(node.orelse)}}

    if isinstance(node, ast.Call):
        return compile_call(node)

    raise CompileError(f"node {type(node).__name__}")


def _field(node: ast.AST) -> dict:
    """`f.get(key[, default])` → field access."""
    if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)):
        raise CompileError("not a method call")
    if node.func.attr != "get":
        raise CompileError(f"method {node.func.attr!r}")
    if not (isinstance(node.func.value, ast.Name) and node.func.value.id == RECV):
        raise CompileError(f"get() on something other than {RECV}")
    if not node.args:
        raise CompileError("get() without key")
    key = node.args[0]
    if not (isinstance(key, ast.Constant) and isinstance(key.value, str)):
        raise CompileError("non-literal get() key")
    if len(node.args) > 2:
        raise CompileError("get() with >2 args")
    default = None
    if len(node.args) == 2:
        # Defaults are written as `-100`, `0`, `""`; fold the constant forms.
        ok, value = _const_value(compile_expr(node.args[1]))
        if not ok:
            raise CompileError("non-constant get() default")
        default = value
    return {"fld": key.value, "dflt": default}


def compile_call(node: ast.Call) -> dict:
    # f.get(...)
    if isinstance(node.func, ast.Attribute) and isinstance(node.func.value, ast.Name) \
            and node.func.value.id == RECV:
        return _field(node)

    if isinstance(node.func, ast.Name):
        name = node.func.id
        if name == "any":
            if len(node.args) != 1 or not isinstance(node.args[0], ast.GeneratorExp):
                raise CompileError("any() without a single genexp")
            gen = node.args[0]
            if len(gen.generators) != 1:
                raise CompileError("any() with multiple generators")
            comp = gen.generators[0]
            if not isinstance(comp.target, ast.Name):
                raise CompileError("any() target is not a name")
            # A filter that mentions the loop variable would need per-item
            # evaluation; reject it rather than silently changing meaning.
            guard = None
            if comp.ifs:
                for cond in comp.ifs:
                    if any(isinstance(n, ast.Name) and n.id == comp.target.id
                           for n in ast.walk(cond)):
                        raise CompileError("any() filter depends on the loop variable")
                guards = [compile_expr(c) for c in comp.ifs]
                guard = guards[0] if len(guards) == 1 else {"and": guards}
            # `any(k in HAY for k in NEEDLES)` — the needles must be literals.
            body = gen.elt
            if not (isinstance(body, ast.Compare) and len(body.ops) == 1
                    and isinstance(body.ops[0], ast.In)):
                raise CompileError("any() body is not a single `in`")
            if not (isinstance(body.left, ast.Name) and body.left.id == comp.target.id):
                raise CompileError("any() body does not test the loop variable")
            needles = compile_expr(comp.iter)
            if "tup" not in needles:
                raise CompileError("any() needles are not a literal sequence")
            return {
                "any_in": {
                    "hay": compile_expr(body.comparators[0]),
                    "needles": [n["lit"] for n in needles["tup"] if "lit" in n],
                    "guard": guard,
                }
            }
        if name == "_peg":
            return {"peg": True}
        if name == "_known_fcf":
            return {"fcf": True}
        if name in ("abs", "max", "min", "str", "bool", "len", "int", "float", "round"):
            return {"call": name, "args": [compile_expr(a) for a in node.args]}
        raise CompileError(f"function {name!r}")

    # Method calls on an arbitrary receiver (`.lower()`, `.strip()`).
    if isinstance(node.func, ast.Attribute):
        meth = node.func.attr
        if meth in ("lower", "upper", "strip"):
            return {"meth": meth, "recv": compile_expr(node.func.value)}
        raise CompileError(f"method {meth!r}")

    raise CompileError("unsupported call")


# Checks that ARE the helper rather than a lambda around it. Their raising
# behaviour is part of the contract (the evaluator turns it into a skip), so
# they map to dedicated spec nodes instead of a transcribed body.
NAMED_CHECKS = {"_known_fcf": {"fcf": True}, "_peg": {"peg": True}}


def compile_rule(check) -> dict:
    global RECV
    named = NAMED_CHECKS.get(getattr(check, "__name__", ""))
    if named is not None:
        return named
    src = inspect.getsource(check).strip()
    tree = ast.parse(src)
    defs = [n for n in ast.walk(tree) if isinstance(n, ast.FunctionDef)]
    RECV = defs[0].args.args[0].arg if defs and defs[0].args.args else "f"
    # Usually `check=lambda f: ...`; `_known_fcf` and `_peg`-style helpers are
    # plain FunctionDefs, so fall back to their return expression.
    lams = [n for n in ast.walk(tree) if isinstance(n, ast.Lambda)]
    if lams:
        return compile_expr(lams[0].body)
    rets = [n for n in ast.walk(tree) if isinstance(n, ast.Return) and n.value is not None]
    if rets:
        return compile_expr(rets[-1].value)
    raise CompileError("no lambda body or return expression")


# ── reference interpreter (mirrors the Rust one) ──────────────────────────


class Unknown(Exception):
    """A rule whose helper raised — the caller skips it."""


def _truthy(v):
    if v is None or v is False:
        return False
    if isinstance(v, (int, float)):
        return v != 0
    if isinstance(v, (str, list, dict, tuple)):
        return len(v) > 0
    return bool(v)


def _peg(f):
    pe = f.get("pe", 0) or 0
    growth = f.get("revenue_growth_latest", 0) or 0
    try:
        if pe <= 0 or growth <= 0:
            return 999
        return pe / growth
    except TypeError:
        return 999


def _known_fcf(f):
    if not f.get("fcf_known"):
        raise Unknown("cash-flow direction unavailable")
    return bool(f.get("fcf_positive"))


def ev(e, f):
    if "lit" in e:
        return e["lit"]
    if "fld" in e:
        return f.get(e["fld"], e["dflt"])
    if "tup" in e:
        return [ev(x, f) for x in e["tup"]]
    # Python's `and`/`or` return the *operand*, not a bool: `25 or 0` is 25, and
    # `(a or b) > 20` therefore compares 25. Boolean `all`/`any` here would make
    # that comparison `True > 20`, silently scoring the rule wrong.
    if "and" in e:
        result = None
        for x in e["and"]:
            result = ev(x, f)
            if not _truthy(result):
                return result
        return result
    if "or" in e:
        result = None
        for x in e["or"]:
            result = ev(x, f)
            if _truthy(result):
                return result
        return result
    if "not" in e:
        return not _truthy(ev(e["not"], f))
    if "neg" in e:
        return -ev(e["neg"], f)
    if "bin" in e:
        b = e["bin"]
        l, r = ev(b["l"], f), ev(b["r"], f)
        try:
            if b["op"] == "add":
                return l + r
            if b["op"] == "sub":
                return l - r
            if b["op"] == "div":
                return l / r
            if b["op"] == "mul":
                return l * r
        except TypeError as exc:
            # The evaluator treats a raising check as "skip"; mirror that.
            raise Unknown(f"bin type error: {exc}") from exc
        raise CompileError(f"bin {b['op']}")
    if "cmp" in e:
        c = e["cmp"]
        l, r = ev(c["l"], f), ev(c["r"], f)
        op = c["op"]
        try:
            if op == "gt": return l > r
            if op == "lt": return l < r
            if op == "ge": return l >= r
            if op == "le": return l <= r
            if op == "eq": return l == r
            if op == "ne": return l != r
            if op == "in":
                return (l in r) if r is not None else False
            if op == "notin":
                return (l not in r) if r is not None else True
            if op == "is": return l is r
            if op == "isnot": return l is not r
        except TypeError as exc:
            raise Unknown(f"cmp type error: {exc}") from exc
        raise CompileError(f"cmp {op}")
    if "if" in e:
        c = e["if"]
        return ev(c["t"], f) if _truthy(ev(c["c"], f)) else ev(c["e"], f)
    if "any_in" in e:
        a = e["any_in"]
        if a.get("guard") is not None and not _truthy(ev(a["guard"], f)):
            return False
        hay = ev(a["hay"], f)
        hay = "" if hay is None else hay
        return any(k in hay for k in a["needles"] if isinstance(k, str) and isinstance(hay, (str, list)))
    if "peg" in e:
        return _peg(f)
    if "fcf" in e:
        return _known_fcf(f)
    if "call" in e:
        args = [ev(a, f) for a in e["args"]]
        n = e["call"]
        try:
            if n == "abs": return abs(args[0])
            if n == "max": return max(args)
            if n == "min": return min(args)
            if n == "str": return str(args[0]) if args else ""
            if n == "bool": return bool(args[0]) if args else False
            if n == "len": return len(args[0])
            if n == "int": return int(args[0])
            if n == "float": return float(args[0])
            if n == "round": return round(*args)
        except (TypeError, ValueError, IndexError):
            return 0
        raise CompileError(f"call {n}")
    if "meth" in e:
        recv = ev(e["recv"], f)
        recv = "" if recv is None else recv
        try:
            return getattr(recv, e["meth"])()
        except AttributeError:
            return ""
    raise CompileError(f"spec node {sorted(e)}")


# ── build ─────────────────────────────────────────────────────────────────

def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    random.seed(20260910)

    groups = []
    for name in sorted(dir(ic)):
        if not name.endswith("_RULES"):
            continue
        val = getattr(ic, name)
        if isinstance(val, list) and val and hasattr(val[0], "check"):
            groups.append((name, val))

    failures: list[str] = []
    rules_out = {}
    total = 0

    # Real feature vector, as the panel stage builds it.
    raw = json.loads((Path.home() / "uzi-ref/cache/raw_data.json").read_text())
    dims = raw.get("dimensions", {})
    real_features = sf.extract_features(raw, dims)

    for gname, rules in groups:
        key = gname[: -len("_RULES")].lower()
        entries = []
        for r in rules:
            total += 1
            try:
                spec = compile_rule(r.check)
            except CompileError as exc:  # pragma: no cover - reported below
                failures.append(f"{gname}.{r.rule_id}: compile: {exc}")
                continue
            entries.append({
                "id": r.rule_id,
                "name": r.name,
                "weight": r.weight,
                "spec": spec,
                "pass_msg": r.pass_msg,
                "fail_msg": r.fail_msg,
            })
        rules_out[key] = entries

    # ── verification ──────────────────────────────────────────────────────
    feature_keys = sorted(real_features.keys())
    vectors = [real_features]
    for _ in range(300):
        v = {}
        for k in feature_keys:
            base = real_features[k]
            if isinstance(base, bool):
                v[k] = random.random() < 0.5
            elif isinstance(base, (int, float)):
                # Stay numeric: a string here would make the original lambda
                # raise TypeError, which tests the harness rather than the port.
                v[k] = random.choice([0, 1, -1, base, base * 0.5, base * 2, 999, 100.0])
            elif isinstance(base, str):
                v[k] = random.choice(["", base, "AI 数据中心", "非多头", "白酒", "abc"])
            elif base is None:
                v[k] = random.choice([None, 0, "", []])
            else:
                v[k] = base
        vectors.append(v)

    checked = 0
    for gname, rules in groups:
        key = gname[: -len("_RULES")].lower()
        for r, entry in zip(rules, rules_out[key]):
            for i, fvec in enumerate(vectors):
                try:
                    want = r.check(fvec)
                    want_err = False
                except Exception:
                    want, want_err = None, True
                try:
                    got = ev(entry["spec"], fvec)
                    got_err = False
                except Unknown:
                    got, got_err = None, True
                except Exception as exc:
                    failures.append(f"{gname}.{r.rule_id}[vec{i}]: spec raised {exc!r}")
                    continue
                checked += 1
                if want_err != got_err:
                    failures.append(
                        f"{gname}.{r.rule_id}[vec{i}]: lambda raised={want_err} but spec raised={got_err}"
                    )
                elif not want_err and _truthy(want) != _truthy(got):
                    failures.append(
                        f"{gname}.{r.rule_id}[vec{i}]: lambda={want!r} spec={got!r}"
                    )

    # Feature vectors + the original lambdas' verdicts, for the Rust-side diff.
    # Without this, a Rust/Python interpreter divergence (e.g. `dict.get`
    # default-vs-null) would pass the in-Python check above unnoticed.
    vectors_out = []
    for fvec in vectors:
        row = {}
        for gname, rules in groups:
            key = gname[: -len("_RULES")].lower()
            for r in rules:
                try:
                    v = r.check(fvec)
                    row[f"{key}.{r.rule_id}"] = {"ok": True, "val": bool(_truthy(v))}
                except Exception:
                    row[f"{key}.{r.rule_id}"] = {"ok": False, "val": None}
        vectors_out.append({"features": fvec, "expected": row})
    (OUT / "rule_vectors.json").write_text(
        json.dumps({"schema": SPEC_VERSION, "vectors": vectors_out}, ensure_ascii=False),
        encoding="utf-8",
    )

    (OUT / "rules.json").write_text(
        json.dumps({"schema": SPEC_VERSION, "groups": rules_out}, ensure_ascii=False, indent=1),
        encoding="utf-8",
    )

    investors = [
        {"id": v["id"], "name": v["name"], "en": v.get("en", ""), "group": v["group"],
         "fields": v.get("fields", []), "source": v.get("source", ""),
         "avatar_seed": v.get("avatar_seed", ""), "tier": v.get("tier")}
        for v in idb.INVESTORS
    ]
    (OUT / "investors.json").write_text(
        json.dumps({"schema": SPEC_VERSION, "investors": investors}, ensure_ascii=False, indent=1),
        encoding="utf-8",
    )

    print(f"investor groups : {len(groups)}")
    print(f"rules compiled  : {sum(len(v) for v in rules_out.values())}/{total}")
    print(f"investors       : {len(investors)}")
    print(f"feature vectors : {len(vectors)} (1 real + {len(vectors) - 1} perturbed)")
    print(f"comparisons     : {checked}")
    print(f"vectors exported: {len(vectors_out)} (for the Rust interpreter diff)")
    print()
    if failures:
        print(f"{len(failures)} FAILURE(S):")
        for f in failures[:40]:
            print("  " + f)
        return 1
    print("ALL RULES MATCH the original lambdas on every vector")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
