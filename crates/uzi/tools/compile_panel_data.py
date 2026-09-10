#!/usr/bin/env python3
"""Dump UZI-Skill's static tables to JSON for the Rust port.

Everything here is *data*, not logic: persona comment templates, authored
investor profiles, market scope, known holdings, industry affinity, LHB seat
ranges, and the investor→rules mapping. Regenerating from the source module is
what keeps the Rust port's tables from drifting.

Note on determinism: `investor_personas.get_comment` picks a line with
`random.choice` and the production path never seeds it, so `comment` fields in
a stored panel.json are not reproducible. This tool therefore exports the whole
candidate list per (investor, signal); the Rust side performs the same choice,
and the differential test compares against the full candidate set rather than a
single expected string.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

SCRIPTS = Path("/tmp/uzi-probe/skills/deep-analysis/scripts")
OUT = Path(__file__).resolve().parent / "out"
sys.path.insert(0, str(SCRIPTS))

import lib.investor_personas as personas  # noqa: E402
import lib.investor_profile as profile  # noqa: E402
import lib.investor_knowledge as knowledge  # noqa: E402
import lib.investor_criteria as criteria  # noqa: E402
import lib.investor_db as idb  # noqa: E402

SCHEMA = 1


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)

    # Literal tables that live as locals inside `extract_features`. Pulling them
    # out as data keeps the Rust port to arithmetic instead of transcribing
    # several hundred keyword strings by hand.
    local_tables = _extract_literal_tables(
        (SCRIPTS / "lib/stock_features.py").read_text(encoding="utf-8"),
        "extract_features",
    )
    print(f"local_tables    : {sorted(local_tables)}")

    doc = {
        "schema": SCHEMA,
        "personas": personas.PERSONAS,
        "persona_fallback": {
            k: v for k, v in personas._GENERIC_FALLBACK.items()
        },
        "profiles": profile.PROFILES,
        "group_default": profile.GROUP_DEFAULT,
        "profile_fallback": profile.GENERIC_FALLBACK,
        "market_scope": knowledge.MARKET_SCOPE,
        "known_holdings": {
            k: [list(t) for t in v] for k, v in knowledge.KNOWN_HOLDINGS.items()
        },
        "industry_affinity": knowledge.INDUSTRY_AFFINITY,
        "feature_tables": local_tables,
    }

    # LHB seats (F-group range gating).
    try:
        import lib.seat_db as seat_db
        doc["seats"] = seat_db.SEATS
    except Exception as exc:  # pragma: no cover
        doc["seats"] = {}
        print(f"WARN: seat_db unavailable ({exc}); F-group range gating disabled")

    # investor → rule group mapping, taken from the real dict so it cannot drift.
    rules_map = {inv: name for name, inv in _investor_rules_pairs(criteria)}
    doc["investor_rule_groups"] = rules_map

    # investor → (group letter, name, mandate) from the DB of record.
    doc["investor_meta"] = {
        v["id"]: {
            "name": v["name"],
            "group": v["group"],
            "mandate": v.get("mandate", "long"),
        }
        for v in idb.INVESTORS
    }

    # Reference feature vector: the single strongest check on the Rust port of
    # `extract_features`, since it exercises every branch at once.
    import lib.stock_features as sf  # noqa: E402
    raw = json.loads((Path.home() / "uzi-ref/cache/raw_data.json").read_text())
    ref_features = sf.extract_features(raw, raw.get("dimensions", {}))
    (OUT / "ref_features.json").write_text(
        json.dumps(ref_features, ensure_ascii=False, indent=1, default=str),
        encoding="utf-8",
    )
    print(f"ref_features    : {len(ref_features)} keys")

    (OUT / "panel_data.json").write_text(
        json.dumps(doc, ensure_ascii=False, indent=1), encoding="utf-8"
    )

    print(f"personas        : {len(doc['personas'])}")
    print(f"profiles        : {len(doc['profiles'])}")
    print(f"group_default   : {len(doc['group_default'])}")
    print(f"market_scope    : {len(doc['market_scope'])}")
    print(f"known_holdings  : {len(doc['known_holdings'])}")
    print(f"industry_affin  : {len(doc['industry_affinity'])}")
    print(f"seats           : {len(doc['seats'])}")
    print(f"investor_rules  : {len(rules_map)} investors")
    return 0


def _extract_literal_tables(source: str, func_name: str) -> dict:
    """Collect simple literal assignments from inside a function body.

    Only names bound to lists/tuples/dicts of literals are returned; anything
    computed is skipped, since those are logic the Rust side must implement.
    """
    import ast

    tree = ast.parse(source)
    fn = next(
        n for n in ast.walk(tree)
        if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)) and n.name == func_name
    )
    tables: dict[str, object] = {}
    for node in fn.body:
        targets = []
        value = None
        if isinstance(node, ast.Assign):
            targets, value = node.targets, node.value
        elif isinstance(node, ast.AnnAssign) and node.value is not None:
            targets, value = [node.target], node.value
        if value is None:
            continue
        for t in targets:
            if not isinstance(t, ast.Name):
                continue
            try:
                literal = ast.literal_eval(value)
            except (ValueError, TypeError):
                continue
            if isinstance(literal, (list, tuple, dict)) and literal:
                tables[t.id] = literal
    return tables


def _investor_rules_pairs(mod):
    """Yield (investor_id, RULES_CONSTANT_NAME) from the module's dict."""
    for inv_id, rules in mod.INVESTOR_RULES.items():
        const = next(
            (n for n in dir(mod) if n.endswith("_RULES") and getattr(mod, n) is rules),
            None,
        )
        yield inv_id, const


if __name__ == "__main__":
    raise SystemExit(main())
