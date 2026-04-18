"""Tests for APEX canonical state engine (FB-CAN-004)."""

from __future__ import annotations

from datetime import UTC, datetime

from app.contracts.canonical_state import DegradationLevel
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.risk import RiskState
from app.contracts.trigger import TriggerOutput
from decision_engine.state_engine import (
    build_canonical_state,
    degradation_size_multiplier,
    merge_canonical_into_risk,
)


def _pkt() -> ForecastPacket:
    return ForecastPacket(
        timestamp=datetime.now(UTC),
        horizons=[1, 3, 5],
        q_low=[-0.01, -0.02, -0.03],
        q_med=[0.0, 0.0, 0.0],
        q_high=[0.01, 0.02, 0.03],
        interval_width=[0.02, 0.04, 0.06],
        regime_vector=[0.4, 0.3, 0.2, 0.1],
        confidence_score=0.8,
        ensemble_variance=[0.01, 0.02, 0.03],
        ood_score=0.1,
    )


def test_build_canonical_state_sums_probabilities():
    feats = {"close": 50_000.0, "atr_14": 100.0, "rsi_14": 50.0}
    apex = build_canonical_state(_pkt(), feats, spread_bps=5.0)
    assert len(apex.regime_probabilities) == 5
    assert abs(sum(apex.regime_probabilities) - 1.0) < 1e-6
    assert 0.0 <= apex.regime_confidence <= 1.0
    assert apex.degradation in DegradationLevel


def test_merge_canonical_into_risk():
    apex = build_canonical_state(_pkt(), {"close": 1.0, "atr_14": 0.0, "rsi_14": 50.0}, spread_bps=1.0)
    r0 = RiskState()
    r1 = merge_canonical_into_risk(r0, apex)
    assert r1.canonical_degradation == apex.degradation
    assert r1.canonical_size_multiplier == degradation_size_multiplier(apex.degradation)


def test_merge_canonical_updates_false_positive_memory():
    apex = build_canonical_state(_pkt(), {"close": 1.0, "atr_14": 0.0, "rsi_14": 50.0}, spread_bps=1.0)
    trig = TriggerOutput(
        setup_valid=True,
        setup_score=0.5,
        pretrigger_valid=True,
        pretrigger_score=0.5,
        trigger_valid=False,
        trigger_type="none",
        trigger_strength=0.4,
        trigger_confidence=0.3,
        missed_move_flag=True,
        trigger_reason_codes=["move_already_extended"],
    )
    r0 = RiskState(trigger_false_positive_memory=0.0)
    r1 = merge_canonical_into_risk(
        r0,
        apex,
        forecast_packet=_pkt(),
        trigger=trig,
        spread_bps=1.0,
        feature_row={"close": 1.0},
    )
    assert r1.trigger_false_positive_memory > 0.0


def test_degradation_no_trade_zero_multiplier():
    assert degradation_size_multiplier(DegradationLevel.NO_TRADE) == 0.0
