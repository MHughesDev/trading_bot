"""Tests for APEX trigger engine (FB-CAN-005)."""

from __future__ import annotations

from dataclasses import replace
from datetime import UTC, datetime

import pytest

from app.contracts.canonical_state import CanonicalStateOutput, DegradationLevel
from app.contracts.reason_codes import (
    TRG_DEGRADATION_BLOCK,
    TRG_INSUFFICIENT_REMAINING_EDGE,
    TRG_MOVE_ALREADY_EXTENDED,
)
from app.contracts.forecast_packet import ForecastPacket
from decision_engine.trigger_engine import evaluate_trigger


def _apex() -> CanonicalStateOutput:
    return CanonicalStateOutput(
        regime_probabilities=[0.25, 0.25, 0.2, 0.15, 0.15],
        regime_confidence=0.4,
        transition_probability=0.2,
        novelty=0.1,
        heat_score=0.2,
        reflexivity_score=0.2,
        degradation=DegradationLevel.NORMAL,
    )


def _pkt() -> ForecastPacket:
    return ForecastPacket(
        timestamp=datetime.now(UTC),
        horizons=[1, 3, 5],
        q_low=[-0.02, -0.03, -0.04],
        q_med=[0.01, 0.0, 0.0],
        q_high=[0.04, 0.05, 0.06],
        interval_width=[0.06, 0.08, 0.1],
        regime_vector=[0.3, 0.3, 0.2, 0.2],
        confidence_score=0.75,
        ensemble_variance=[0.01, 0.02, 0.03],
        ood_score=0.05,
    )


def test_evaluate_trigger_emits_stages():
    feats = {"close": 50_000.0, "rsi_14": 55.0, "return_1": 0.001, "volume": 1e6}
    out = evaluate_trigger(_pkt(), feats, spread_bps=5.0, apex=_apex())
    assert hasattr(out, "setup_valid")
    assert 0.0 <= out.setup_score <= 1.0
    assert 0.0 <= out.pretrigger_score <= 1.0
    assert out.trigger_type in (
        "none",
        "imbalance_spike",
        "volume_burst",
        "structure_break",
        "composite_confirmed",
    )
    assert out.stage_timestamp_setup and out.stage_timestamp_pretrigger and out.stage_timestamp_confirm
    assert out.setup_to_confirm_latency_ms == pytest.approx(2.0)
    assert "setup" in out.stage_failure_codes


def test_trigger_stage_failure_codes_when_setup_fails():
    apex = _apex().model_copy(update={"degradation": DegradationLevel.NO_TRADE})
    out = evaluate_trigger(_pkt(), {"close": 1.0}, spread_bps=1.0, apex=apex)
    assert out.stage_failure_codes.get("setup")


def test_no_trade_degradation_blocks():
    apex = _apex().model_copy(update={"degradation": DegradationLevel.NO_TRADE})
    out = evaluate_trigger(_pkt(), {"close": 1.0}, spread_bps=1.0, apex=apex)
    assert out.setup_valid is False
    assert TRG_DEGRADATION_BLOCK in out.trigger_reason_codes


def test_missed_move_suppresses_after_strength_met():
    """§8: extension / remaining-edge gate runs when strength would otherwise confirm."""
    pkt = replace(_pkt(), interval_width=[0.5, 0.08, 0.1])
    feats = {"close": 50_000.0, "rsi_14": 55.0, "return_1": 0.02, "volume": 1e6}
    out = evaluate_trigger(pkt, feats, spread_bps=5.0, apex=_apex())
    assert out.missed_move_flag is True
    assert out.trigger_valid is False
    assert TRG_MOVE_ALREADY_EXTENDED in out.trigger_reason_codes


def test_remaining_edge_missed_sets_insufficient_remaining_edge():
    pkt = replace(_pkt(), interval_width=[0.02, 0.04, 0.06])
    feats = {"close": 50_000.0, "rsi_14": 55.0, "return_1": 0.001, "volume": 1e6}
    out = evaluate_trigger(pkt, feats, spread_bps=75.0, apex=_apex())
    assert out.missed_move_flag is True
    assert TRG_INSUFFICIENT_REMAINING_EDGE in out.trigger_reason_codes
