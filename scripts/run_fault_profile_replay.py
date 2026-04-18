#!/usr/bin/env python3
"""Run a minimal deterministic replay under a named canonical fault profile (FB-CAN-037).

Prints a suggested replay_run_id and JSON line suitable for release evidence bundles.
"""

from __future__ import annotations

import argparse
import json
from datetime import UTC, datetime

import polars as pl

from app.config.settings import AppSettings
from app.contracts.replay_events import ReplayRunContract
from backtesting.replay import replay_decisions
from decision_engine.pipeline import DecisionPipeline
from orchestration.fault_injection_profiles import list_canonical_fault_profile_ids
from risk_engine.engine import RiskEngine


def _bars(n: int = 16) -> pl.DataFrame:
    rows = []
    for i in range(n):
        t = datetime(2026, 1, 1, 0, i, 0, tzinfo=UTC)
        p = 100.0 + i * 0.1
        rows.append(
            {"timestamp": t, "open": p, "high": p + 0.05, "low": p - 0.05, "close": p, "volume": 1.0}
        )
    return pl.DataFrame(rows)


def main() -> int:
    p = argparse.ArgumentParser(description="Canonical fault-profile replay smoke (FB-CAN-037)")
    p.add_argument(
        "--profile-id",
        required=True,
        choices=list(list_canonical_fault_profile_ids()),
        help="Canonical fault_injection_profile_id",
    )
    p.add_argument(
        "--replay-run-id",
        default="",
        help="Override replay_run_id (default: ci-fault-<profile_id>)",
    )
    args = p.parse_args()
    rid = args.replay_run_id.strip() or f"ci-fault-{args.profile_id}"

    df = _bars()
    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    contract = ReplayRunContract(
        replay_run_id=rid,
        dataset_id="ci-fault",
        config_version="1.0.0",
        logic_version="1.0.0",
        instrument_scope=["BTC-USD"],
        replay_mode="synthetic_fault_injected",
        fault_injection_profile_id=args.profile_id,
    )
    out = replay_decisions(
        df,
        pipe,
        eng,
        symbol="BTC-USD",
        spread_bps=5.0,
        replay_contract=contract,
        emit_canonical_events=True,
    )
    last = out[-1]
    ev = last.get("canonical_events") or []
    fault_n = sum(1 for e in ev if e.get("event_family") == "fault_injection_event")
    summary = {
        "replay_run_id": rid,
        "fault_injection_profile_id": args.profile_id,
        "bars": len(out),
        "fault_injection_events": fault_n,
        "ok": len(out) == 16 and fault_n > 0,
    }
    print(json.dumps(summary, indent=2))
    if not summary["ok"]:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
