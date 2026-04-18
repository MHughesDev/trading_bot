"""Tests for canonical replay interface (FB-CAN-009)."""

from __future__ import annotations

from datetime import UTC, datetime

import polars as pl

from app.config.settings import AppSettings
from app.contracts.replay_events import ReplayRunContract
from backtesting.replay import replay_decisions
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine


def test_replay_emits_canonical_events_when_requested():
    rows = []
    for i in range(12):
        t = datetime(2025, 1, 1, 0, i, 0, tzinfo=UTC)
        p = 100.0 + i * 0.1
        rows.append(
            {"timestamp": t, "open": p, "high": p + 0.05, "low": p - 0.05, "close": p, "volume": 1.0}
        )
    df = pl.DataFrame(rows)
    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    contract = ReplayRunContract(
        replay_run_id="test-run-1",
        dataset_id="unit",
        config_version="9.9.9",
        logic_version="9.9.9",
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
    assert len(out) == 12
    ev = out[-1].get("canonical_events")
    assert ev is not None
    families = {e["event_family"] for e in ev}
    assert "market_snapshot_event" in families
    assert "decision_output_event" in families


def test_fault_injection_widens_spread():
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
    contract = ReplayRunContract(replay_run_id="f1", dataset_id="u", instrument_scope=["X"])
    out = replay_decisions(
        df,
        pipe,
        eng,
        symbol="X",
        spread_bps=10.0,
        replay_contract=contract,
        emit_canonical_events=True,
        fault_injection_profile={"spread_widen_mult": 3.0},
    )
    ev = out[-1]["canonical_events"]
    fault_ev = [e for e in ev if e["event_family"] == "fault_injection_event"]
    assert fault_ev
