#!/usr/bin/env python3
"""FB-CAN-025: replay with ``emit_canonical_events`` must be deterministic for fixed inputs."""
from __future__ import annotations

import hashlib
import json
import sys
from datetime import UTC, datetime

import polars as pl

from app.config.settings import AppSettings
from app.contracts.replay_events import ReplayRunContract
from backtesting.replay import replay_decisions
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine


def _run_once() -> str:
    rows = []
    for i in range(8):
        t = datetime(2025, 1, 1, 0, i, 0, tzinfo=UTC)
        p = 100.0 + i * 0.1
        rows.append(
            {"timestamp": t, "open": p, "high": p + 0.05, "low": p - 0.05, "close": p, "volume": 1.0}
        )
    df = pl.DataFrame(rows)
    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    contract = ReplayRunContract(
        replay_run_id="ci-determinism",
        dataset_id="unit",
        config_version="1.0.0",
        logic_version="1.0.0",
        instrument_scope=["BTC-USD"],
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
    ev = out[-1].get("canonical_events")
    if ev is None:
        print("ci_replay_determinism: last row missing canonical_events", file=sys.stderr)
        raise SystemExit(1)
    payload = json.dumps(ev, sort_keys=True, default=str)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def main() -> int:
    a = _run_once()
    b = _run_once()
    if a != b:
        print("ci_replay_determinism: two replay runs produced different canonical_events hashes", file=sys.stderr)
        print(f"  {a!s} != {b!s}", file=sys.stderr)
        return 1
    print("ci_replay_determinism: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
