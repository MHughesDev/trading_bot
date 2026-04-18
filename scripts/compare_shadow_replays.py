#!/usr/bin/env python3
"""Compare two replay runs for shadow promotion evidence (FB-CAN-038).

Runs ``replay_decisions`` twice (baseline vs candidate logic_version), compares per-bar
decision outputs, prints JSON report, and optionally saves to ``models/registry/shadow_comparison_store.json``.
"""

from __future__ import annotations

import argparse
import json
import sys
from datetime import UTC, datetime, timedelta

import polars as pl

from app.config.settings import AppSettings, load_settings
from app.contracts.replay_events import ReplayRunContract
from app.config.shadow_comparison import shadow_policy_from_settings
from backtesting.replay import replay_decisions
from decision_engine.pipeline import DecisionPipeline
from models.registry.shadow_comparison_store import save_shadow_comparison_report
from orchestration.shadow_comparison import compare_shadow_replay_rows
from risk_engine.engine import RiskEngine


def _bars(n: int) -> pl.DataFrame:
    rows = []
    base = datetime(2026, 6, 1, 0, 0, 0, tzinfo=UTC)
    for i in range(n):
        t = base + timedelta(seconds=i)
        p = 100.0 + i * 0.05
        rows.append(
            {"timestamp": t, "open": p, "high": p + 0.02, "low": p - 0.02, "close": p, "volume": 1.0}
        )
    return pl.DataFrame(rows)


def main() -> int:
    p = argparse.ArgumentParser(description="Shadow vs baseline replay comparison")
    p.add_argument("--bars", type=int, default=220, help="Synthetic bar count (default >= probation min_bars)")
    p.add_argument("--symbol", default="BTC-USD")
    p.add_argument("--baseline-run-id", default="shadow-baseline")
    p.add_argument("--candidate-run-id", default="shadow-candidate")
    p.add_argument("--baseline-logic-version", default="1.0.0")
    p.add_argument("--candidate-logic-version", default="1.0.1")
    p.add_argument("--save-store", action="store_true", help="Write models/registry/shadow_comparison_store.json")
    p.add_argument("--out", type=str, default="", help="Write JSON report to path")
    args = p.parse_args()

    settings = load_settings()
    policy = shadow_policy_from_settings(settings)
    if not policy.enabled:
        print("shadow_comparison.enabled is false in policy", file=sys.stderr)
        return 1

    df = _bars(args.bars)
    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())

    base_c = ReplayRunContract(
        replay_run_id=args.baseline_run_id,
        dataset_id="shadow-compare",
        config_version=str(settings.canonical.metadata.config_version),
        logic_version=args.baseline_logic_version,
        instrument_scope=[args.symbol],
        replay_mode="shadow_comparison",
    )
    cand_c = ReplayRunContract(
        replay_run_id=args.candidate_run_id,
        dataset_id="shadow-compare",
        config_version=str(settings.canonical.metadata.config_version),
        logic_version=args.candidate_logic_version,
        instrument_scope=[args.symbol],
        replay_mode="shadow_comparison",
    )

    base_out = replay_decisions(
        df,
        pipe,
        eng,
        symbol=args.symbol,
        spread_bps=5.0,
        replay_contract=base_c,
        emit_canonical_events=True,
    )
    pipe2 = DecisionPipeline()
    eng2 = RiskEngine(AppSettings())
    cand_out = replay_decisions(
        df,
        pipe2,
        eng2,
        symbol=args.symbol,
        spread_bps=5.0,
        replay_contract=cand_c,
        emit_canonical_events=True,
    )

    rep = compare_shadow_replay_rows(
        base_out,
        cand_out,
        policy=policy,
        baseline_replay_run_id=args.baseline_run_id,
        candidate_replay_run_id=args.candidate_run_id,
        config_version=str(settings.canonical.metadata.config_version),
        logic_version_baseline=args.baseline_logic_version,
        logic_version_candidate=args.candidate_logic_version,
    )
    passed = bool(rep.within_thresholds and rep.probation_passed)
    out = rep.to_dict()
    out["shadow_comparison_passed"] = passed

    text = json.dumps(out, indent=2)
    if args.out:
        from pathlib import Path

        Path(args.out).write_text(text, encoding="utf-8")
    print(text)
    if args.save_store:
        save_shadow_comparison_report(out)
        print("wrote shadow comparison store", file=sys.stderr)
    return 0 if passed else 2


if __name__ == "__main__":
    raise SystemExit(main())
