"""FB-CAN-055: replay mode ↔ canonical event-family coverage."""

from __future__ import annotations

from datetime import UTC, datetime

import polars as pl
import pytest

from app.config.settings import AppSettings
from app.contracts.replay_events import ReplayMode, ReplayRunContract
from backtesting.replay import replay_decisions
from backtesting.replay_coverage import (
    collect_event_families_from_replay_rows,
    validate_replay_event_family_coverage,
)
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine


def _df(n: int = 8) -> pl.DataFrame:
    rows = []
    for i in range(n):
        t = datetime(2025, 1, 1, 0, i, 0, tzinfo=UTC)
        p = 100.0 + i * 0.1
        rows.append(
            {"timestamp": t, "open": p, "high": p + 0.05, "low": p - 0.05, "close": p, "volume": 1.0}
        )
    return pl.DataFrame(rows)


def test_collect_families_unions_multi_asset_symbols():
    row = {
        "timestamp": datetime.now(UTC),
        "symbols": {
            "A": {
                "canonical_events": [
                    {"event_family": "market_snapshot_event"},
                    {"event_family": "decision_output_event"},
                ]
            }
        },
    }
    assert collect_event_families_from_replay_rows([row]) == {"market_snapshot_event", "decision_output_event"}


def test_synthetic_fault_injected_requires_fault_family():
    df = _df()
    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    contract = ReplayRunContract(
        replay_run_id="syn-bad",
        dataset_id="u",
        instrument_scope=["X"],
        replay_mode=ReplayMode.SYNTHETIC_FAULT_INJECTED,
    )
    with pytest.raises(ValueError, match="fault_injection_event"):
        replay_decisions(
            df,
            pipe,
            eng,
            symbol="X",
            replay_contract=contract,
            emit_canonical_events=True,
        )


def test_synthetic_fault_injected_passes_with_named_profile():
    df = _df()
    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    contract = ReplayRunContract(
        replay_run_id="syn-ok",
        dataset_id="u",
        instrument_scope=["BTC-USD"],
        replay_mode=ReplayMode.SYNTHETIC_FAULT_INJECTED,
        fault_injection_profile_id="spread_widening_stress",
    )
    out = replay_decisions(
        df,
        pipe,
        eng,
        symbol="BTC-USD",
        replay_contract=contract,
        emit_canonical_events=True,
    )
    assert len(out) == 8
    ok, reasons = validate_replay_event_family_coverage(out, contract, emit_canonical_events=True)
    assert ok, reasons


def test_execution_debug_skips_exec_feedback_when_no_trade():
    df = _df()
    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    contract = ReplayRunContract(
        replay_run_id="exec-dbg",
        dataset_id="u",
        instrument_scope=["BTC-USD"],
        replay_mode=ReplayMode.EXECUTION_DEBUG,
    )
    out = replay_decisions(
        df,
        pipe,
        eng,
        symbol="BTC-USD",
        replay_contract=contract,
        emit_canonical_events=True,
    )
    ok, reasons = validate_replay_event_family_coverage(out, contract, emit_canonical_events=True)
    assert ok, reasons


def test_validate_skips_when_emit_false():
    contract = ReplayRunContract(replay_run_id="x", dataset_id="d", instrument_scope=["S"])
    ok, reasons = validate_replay_event_family_coverage([], contract, emit_canonical_events=False)
    assert ok and reasons == []
