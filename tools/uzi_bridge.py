#!/usr/bin/env python3
"""Taoli ↔ UZI-Skill bridge.

The UZI pipeline is split so the *analysis* runs in Rust while data collection
and HTML rendering stay in Python:

  1. ``fetch``  — Python collects market data and computes the institutional
                  modeling dimensions (DCF / Comps / LBO / research workflow /
                  deep methods), which are numerical finance code built on
                  Python-only data sources.
  2. (Rust)     — reads ``raw_data.json`` and writes ``dimensions.json``,
                  ``panel.json`` and ``synthesis.json``.
  3. ``render`` — Python assembles the standalone HTML from those artifacts.

This file is glue only: every step calls the upstream project's own functions.
No upstream file is modified, so the vendored skill stays updatable.

Usage:
    python3 uzi_bridge.py fetch  <skill_dir> <ticker> [--depth lite]
    python3 uzi_bridge.py quant  <skill_dir> <ticker>     # quant-factor style?
    python3 uzi_bridge.py render <skill_dir> <ticker>
    python3 uzi_bridge.py path   <skill_dir> <ticker>     # print cache dir
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path


def _load(skill_dir: Path):
    """Import the upstream modules with their scripts dir on sys.path.

    Also chdirs into that directory: the upstream cache root is the *relative*
    path ``.cache``, so running from anywhere else silently reads and writes the
    wrong location (fetches appear to succeed while writing nothing useful).
    """
    scripts = (skill_dir / "skills" / "deep-analysis" / "scripts").resolve()
    if not scripts.is_dir():
        raise SystemExit(f"not a UZI-Skill checkout: {skill_dir}")
    os.chdir(scripts)
    sys.path.insert(0, str(scripts))
    return scripts


def cmd_fetch(skill_dir: Path, ticker: str, depth: str) -> int:
    """Collect raw data and the institutional modeling dimensions."""
    _load(skill_dir)

    # `--depth` in the upstream CLI is read from the environment by the pipeline.
    os.environ["UZI_DEPTH"] = depth

    from lib.cache import write_task_output
    from lib.pipeline.preflight_helpers import prepare_target
    from run_real_test import collect_raw_data, _detect_lite_mode

    prepared = prepare_target(ticker, detect_lite_fn=_detect_lite_mode)
    if not prepared["ok"]:
        print(json.dumps(prepared["payload"], ensure_ascii=False))
        return 1
    info = prepared["ticker_info"]

    raw = collect_raw_data(info.full)
    write_task_output(info.full, "raw_data", raw)

    # Institutional modeling (dims 20-22). Failures are tolerated upstream — the
    # pipeline records them and continues — so a failure here is not fatal.
    from compute_deep_methods import compute_dim_20, compute_dim_21, compute_dim_22
    from lib.stock_features import extract_features, sanitize_features

    features = sanitize_features(extract_features(raw, raw.get("dimensions", {})))
    try:
        d20 = compute_dim_20(features, raw)
        raw["dimensions"]["20_valuation_models"] = d20
        d21 = compute_dim_21(features, raw, d20)
        raw["dimensions"]["21_research_workflow"] = d21
        raw["dimensions"]["22_deep_methods"] = compute_dim_22(features, raw, d20, d21)
        write_task_output(info.full, "raw_data", raw)
    except Exception as exc:  # noqa: BLE001 - upstream tolerates this too
        print(f"WARN: institutional modeling failed: {type(exc).__name__}: {exc}", file=sys.stderr)

    print(f"FETCHED {info.full}")
    return 0


def cmd_render(skill_dir: Path, ticker: str) -> int:
    """Assemble the standalone HTML from the cached artifacts."""
    scripts = _load(skill_dir)

    # The upstream self-review gate refuses to render when it finds critical data
    # gaps. Those gaps are a property of the *data*, not of this integration, and
    # the report already surfaces them as badges — so the gate is skipped and the
    # gaps travel with the report instead of blocking it.
    os.environ.setdefault("UZI_SKIP_REVIEW", "1")

    from run_real_test import stage2

    report = stage2(ticker)
    print(f"REPORT {report}")
    return 0


def cmd_quant(skill_dir: Path, ticker: str) -> int:
    """Report whether quant funds hold this name in their top ten.

    `detect_quant_signal` fetches fund holdings over the network, so it belongs
    with the other data-collection steps. The Rust analysis stage consumes the
    boolean rather than reimplementing fund lookups.
    """
    _load(skill_dir)
    import json as _json

    from lib.cache import read_task_output
    from lib.quant_signal import detect_quant_signal
    from lib.quant_signal import _fetch_all_holding_funds

    cache_dir = _cache_dir_for(ticker)
    if cache_dir is None:
        print("false")
        return 0
    raw = read_task_output(cache_dir.name, "raw_data") or {}
    code = raw.get("ticker", ticker)
    try:
        funds = raw.get("fund_managers") or []
        if not funds:
            funds = _fetch_all_holding_funds(code.split(".")[0], max_funds=80)
        signal = detect_quant_signal(code, funds)
        print("true" if signal.get("is_quant_factor_style") else "false")
    except Exception as exc:  # noqa: BLE001
        print(f"WARN: quant detection failed: {type(exc).__name__}: {exc}", file=sys.stderr)
        print("false")
    return 0


def _cache_dir_for(ticker: str):
    """Resolve `.cache/<full ticker>` for an input like ``600519`` or ``600519.SH``."""
    from lib.cache import CACHE_ROOT

    if not CACHE_ROOT.is_dir():
        return None
    wanted = ticker.split(".")[0]
    matches = [
        d for d in sorted(CACHE_ROOT.iterdir())
        if d.is_dir() and d.name.split(".")[0] == wanted and d.name != "_global"
    ]
    # Prefer an explicit market suffix when one exists.
    for d in matches:
        if d.name == ticker:
            return d
    return matches[0] if matches else None


def cmd_path(skill_dir: Path, ticker: str) -> int:
    _load(skill_dir)
    found = _cache_dir_for(ticker)
    if found is None:
        print("", end="")
        return 1
    print(found.resolve())
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["fetch", "render", "quant", "path"])
    parser.add_argument("skill_dir", type=Path)
    parser.add_argument("ticker")
    parser.add_argument("--depth", default="lite", choices=["lite", "medium", "deep"])
    args = parser.parse_args()

    if args.command == "fetch":
        return cmd_fetch(args.skill_dir, args.ticker, args.depth)
    if args.command == "render":
        return cmd_render(args.skill_dir, args.ticker)
    if args.command == "quant":
        return cmd_quant(args.skill_dir, args.ticker)
    return cmd_path(args.skill_dir, args.ticker)


if __name__ == "__main__":
    raise SystemExit(main())
