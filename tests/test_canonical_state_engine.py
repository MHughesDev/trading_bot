"""Tests for APEX canonical state engine (FB-CAN-004)."""

from __future__ import annotations

from datetime import UTC, datetime

import pytest

from app.config.settings import AppSettings
from app.contracts.canonical_state import DegradationLevel
from app.contracts.reason_codes import TRG_MOVE_ALREADY_EXTENDED
from app.contracts.canonical_structure import CanonicalStructureOutput
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
    rp = sorted(apex.regime_probabilities, reverse=True)
    assert apex.regime_confidence == pytest.approx(rp[0] - rp[1], rel=1e-9, abs=1e-9)
    assert 0.0 <= apex.transition_probability <= 1.0


def test_build_canonical_state_includes_novelty_trace_with_structure():
    st = CanonicalStructureOutput(
        p05=-0.02,
        p25=-0.01,
        p50=0.0,
        p75=0.01,
        p95=0.02,
        volatility_forecast=0.02,
        asymmetry_score=0.3,
        continuation_probability=0.5,
        fragility_score=0.8,
        directional_bias=0.1,
        model_agreement_score=0.7,
        model_correlation_penalty=0.2,
    )
    feats = {"close": 50_000.0, "atr_14": 100.0, "rsi_14": 50.0}
    apex = build_canonical_state(
        _pkt(),
        feats,
        spread_bps=5.0,
        settings=AppSettings(),
        structure=st,
    )
    assert "ood" in apex.novelty_components
    assert "rsi_ext" in apex.reflexivity_components
    assert isinstance(apex.novelty_reason_codes, list)


def test_regime_confidence_matches_spec_separation():
    """APEX State spec §6 — max(R) - second_max(R) on the 5-class vector."""
    from decision_engine.state_engine import _regime_confidence_separation

    assert _regime_confidence_separation([0.5, 0.3, 0.1, 0.05, 0.05]) == pytest.approx(0.2)


def test_merge_canonical_into_risk():
    from app.config.settings import AppSettings

    apex = build_canonical_state(_pkt(), {"close": 1.0, "atr_14": 0.0, "rsi_14": 50.0}, spread_bps=1.0)
    r0 = RiskState()
    r1 = merge_canonical_into_risk(r0, apex, settings=AppSettings())
    assert r1.canonical_degradation == apex.degradation
    base = degradation_size_multiplier(apex.degradation)
    assert r1.canonical_size_multiplier <= base + 1e-9
    assert r1.canonical_degradation_sizing_terms is not None
    assert r1.canonical_degradation_sizing_terms.get("degradation_base_multiplier") == pytest.approx(base)


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
        trigger_reason_codes=[TRG_MOVE_ALREADY_EXTENDED],
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
