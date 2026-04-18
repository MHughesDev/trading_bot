#!/usr/bin/env python3
"""FB-CAN-030: two replays with identical contract must yield matching decision fingerprints."""
from __future__ import annotations

import sys
from datetime import UTC, datetime

import polars as pl

from app.config.settings import AppSettings
from app.contracts.replay_events import ReplayRunContract
from backtesting.live_replay_equivalence import (
    compare_decision_fingerprint_sequences,
    fingerprints_from_replay_rows,
)
from backtesting.replay import replay_decisions
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine


def _bars() -> pl.DataFrame:
    rows = []
    for i in range(8):
        t = datetime(2025, 1, 1, 0, i, 0, tzinfo=UTC)
        p = 100.0 + i * 0.1
        rows.append(
            {"timestamp": t, "open": p, "high": p + 0.05, "low": p - 0.05, "close": p, "volume": 1.0}
        )
    return pl.DataFrame(rows)


def _run() -> list[str]:
    df = _bars()
    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    contract = ReplayRunContract(
        replay_run_id="ci-equiv",
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
    return fingerprints_from_replay_rows(out)


def main() -> int:
    a = _run()
    b = _run()
    rep = compare_decision_fingerprint_sequences(a, b)
    if not rep.equivalent:
        print(
            "ci_live_replay_equivalence: two replays diverged",
            rep.to_dict(),
            file=sys.stderr,
        )
        return 1
    print("ci_live_replay_equivalence: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
