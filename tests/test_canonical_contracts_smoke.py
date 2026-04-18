"""Round-trip smoke tests for APEX canonical contracts (FB-CAN-013).

Keeps pydantic schemas stable for CI; domain logic lives in dedicated test modules.
"""

from __future__ import annotations

from app.contracts.auction import AuctionCandidateRecord, AuctionResult
from app.contracts.execution_guidance import ExecutionFeedback, ExecutionGuidance
from app.contracts.replay_events import ReplayEventEnvelope, ReplayRunContract
from app.contracts.trigger import TriggerOutput


def test_trigger_output_roundtrip():
    t = TriggerOutput(
        setup_valid=True,
        setup_score=0.5,
        pretrigger_valid=True,
        pretrigger_score=0.4,
        trigger_valid=True,
        trigger_type="composite_confirmed",
        trigger_strength=0.6,
        trigger_confidence=0.55,
        missed_move_flag=False,
        trigger_reason_codes=["ok"],
    )
    t2 = TriggerOutput.model_validate(t.model_dump(mode="json"))
    assert t2.trigger_valid is True


def test_auction_result_roundtrip():
    r = AuctionResult(
        selected_symbol="BTC-USD",
        selected_direction=1,
        selected_score=0.7,
        records=[
            AuctionCandidateRecord(
                symbol="BTC-USD",
                direction=1,
                eligible=True,
                status="selected",
                auction_score=0.7,
                components={"x": 1.0},
                penalties={"p": 0.1},
                reasons=["ok"],
            )
        ],
    )
    r2 = AuctionResult.model_validate(r.model_dump(mode="json"))
    assert r2.selected_symbol == "BTC-USD"


def test_replay_contracts_roundtrip():
    c = ReplayRunContract(
        replay_run_id="smoke-1",
        dataset_id="d",
        config_version="2.0.0",
        logic_version="2.0.0",
        instrument_scope=["BTC-USD"],
    )
    c2 = ReplayRunContract.model_validate(c.model_dump(mode="json"))
    assert c2.replay_run_id == "smoke-1"

    env = ReplayEventEnvelope(
        event_family="decision_output_event",
        replay_run_id="smoke-1",
        symbol="BTC-USD",
        payload={"k": 1},
    )
    env2 = ReplayEventEnvelope.model_validate(env.model_dump(mode="json"))
    assert env2.event_family == "decision_output_event"


def test_execution_guidance_roundtrip():
    g = ExecutionGuidance(
        preferred_execution_style="passive",
        execution_confidence=0.8,
        max_slippage_tolerance_bps=12.0,
        stress_mode_flag=False,
        execution_reason_codes=["ok"],
        style_rationale_codes=["style_branch_passive_high_conf_tight_spread"],
        worst_case_edge=0.0,
        remaining_edge=0.01,
        urgency_high=False,
        suppress_order=False,
        size_multiplier=1.0,
    )
    g2 = ExecutionGuidance.model_validate(g.model_dump(mode="json"))
    assert g2.preferred_execution_style == "passive"

    fb = ExecutionFeedback(
        fill_ratio=0.95,
        realized_slippage_bps=2.0,
        venue_quality_score=0.9,
        partial_fill_flag=True,
        adapter="alpaca_paper",
    )
    fb2 = ExecutionFeedback.model_validate(fb.model_dump(mode="json"))
    assert fb2.partial_fill_flag is True
