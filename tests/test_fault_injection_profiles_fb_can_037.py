"""FB-CAN-037: canonical fault profiles and replay merge."""

from __future__ import annotations

from datetime import UTC, datetime

import polars as pl

from app.config.settings import AppSettings
from app.contracts.replay_events import ReplayRunContract
from backtesting.fault_injection import apply_fault_injection
from backtesting.replay import replay_decisions
from decision_engine.pipeline import DecisionPipeline
from orchestration.fault_injection_profiles import (
    CANONICAL_FAULT_PROFILES,
    fault_stress_evidence_satisfied,
    list_canonical_fault_profile_ids,
    merge_replay_fault_profile,
    resolve_fault_profile_dict,
)
from risk_engine.engine import RiskEngine


def test_list_and_resolve_profiles():
    ids = list_canonical_fault_profile_ids()
    assert "spread_widening_stress" in ids
    assert resolve_fault_profile_dict("spread_widening_stress") == CANONICAL_FAULT_PROFILES[
        "spread_widening_stress"
    ]
    assert resolve_fault_profile_dict("unknown") == {}


def test_merge_order_named_then_contract_then_kwarg():
    m = merge_replay_fault_profile(
        fault_injection_profile_id="spread_widening_stress",
        contract_profile={"spread_widen_mult": 2.0},
        override={"spread_widen_mult": 10.0},
    )
    assert m["spread_widen_mult"] == 10.0
    assert "spread_widening_stress" in CANONICAL_FAULT_PROFILES


def test_fault_stress_evidence_satisfied():
    all_ids = list(list_canonical_fault_profile_ids())
    assert fault_stress_evidence_satisfied(
        fault_stress_run_ids=["r1"],
        fault_profile_ids_satisfied=all_ids,
    )
    assert not fault_stress_evidence_satisfied(
        fault_stress_run_ids=[],
        fault_profile_ids_satisfied=all_ids,
    )
    assert not fault_stress_evidence_satisfied(
        fault_stress_run_ids=["r1"],
        fault_profile_ids_satisfied=all_ids[:-1],
    )


def test_apply_fault_book_imbalance_and_execution():
    row = {"rsi_14": 50.0, "canonical_exec_fill_ratio": 1.0, "canonical_exec_slippage_bps": 0.0}
    feats, sp, dt, reasons = apply_fault_injection(
        feature_row=row,
        spread_bps=10.0,
        data_timestamp=datetime(2026, 1, 1, tzinfo=UTC),
        profile={"book_imbalance_bias": 0.9, "execution_slippage_bps_add": 20.0},
    )
    assert feats["rsi_14"] > 80.0
    assert feats["canonical_exec_slippage_bps"] == 20.0
    assert "book_imbalance_injection" in reasons
    assert "execution_slippage_stress_injection" in reasons


def test_replay_contract_named_profile_emits_fault_events():
    rows = []
    for i in range(4):
        t = datetime(2026, 1, 1, 0, i, 0, tzinfo=UTC)
        p = 100.0 + i * 0.1
        rows.append(
            {"timestamp": t, "open": p, "high": p + 0.05, "low": p - 0.05, "close": p, "volume": 1.0}
        )
    df = pl.DataFrame(rows)
    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    contract = ReplayRunContract(
        replay_run_id="t-fault",
        dataset_id="u",
        instrument_scope=["X"],
        fault_injection_profile_id="spread_widening_stress",
    )
    out = replay_decisions(
        df,
        pipe,
        eng,
        symbol="X",
        spread_bps=10.0,
        replay_contract=contract,
        emit_canonical_events=True,
    )
    ev = out[-1]["canonical_events"]
    fault = [e for e in ev if e["event_family"] == "fault_injection_event"]
    assert fault
    payload = fault[-1]["payload"]
    assert payload.get("profile", {}).get("spread_widen_mult") == 4.5
