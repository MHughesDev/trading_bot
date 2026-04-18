"""Carry sleeve domain (FB-CAN-018)."""

from __future__ import annotations

from app.contracts.canonical_state import CanonicalStateOutput, DegradationLevel
from app.contracts.decisions import ActionProposal, RouteId
from app.contracts.trigger import TriggerOutput
from carry_sleeve.config import CarrySleeveConfig
from carry_sleeve.engine import (
    build_carry_proposal,
    evaluate_carry_sleeve,
    funding_signal_from_features,
)


def _apex() -> CanonicalStateOutput:
    return CanonicalStateOutput(
        regime_probabilities=[0.2, 0.2, 0.2, 0.2, 0.2],
        regime_confidence=0.5,
        transition_probability=0.1,
        novelty=0.1,
        heat_score=0.2,
        reflexivity_score=0.2,
        degradation=DegradationLevel.NORMAL,
    )


def _trig(conf: float) -> TriggerOutput:
    return TriggerOutput(
        setup_valid=True,
        setup_score=0.5,
        pretrigger_valid=True,
        pretrigger_score=0.5,
        trigger_valid=True,
        trigger_type="composite_confirmed",
        trigger_confidence=conf,
        trigger_strength=0.5,
        missed_move_flag=False,
        trigger_reason_codes=[],
    )


def test_funding_signal_from_zscore():
    fs = funding_signal_from_features({"funding_rate_zscore": 2.0})
    assert fs == 0.5


def test_carry_disabled_by_default():
    row = {"close": 100.0, "funding_rate_zscore": 4.0}
    dec = evaluate_carry_sleeve(
        row,
        _trig(0.05),
        _apex(),
        CarrySleeveConfig(),
        directional_proposal=None,
    )
    assert dec.active is False
    assert "carry_disabled" in dec.reason_codes


def test_carry_active_neutral_path():
    cfg = CarrySleeveConfig(
        carry_enabled=True,
        carry_funding_threshold=0.2,
        carry_max_exposure_usd=10_000.0,
        carry_independent_risk_multiplier=0.5,
    )
    row = {"close": 100.0, "funding_rate_zscore": 3.0}
    dec = evaluate_carry_sleeve(
        row,
        _trig(0.05),
        _apex(),
        cfg,
        directional_proposal=None,
    )
    assert dec.active is True
    assert dec.target_notional_usd == 5000.0


def test_carry_suppresses_directional_when_isolation():
    cfg = CarrySleeveConfig(
        carry_enabled=True,
        carry_funding_threshold=0.2,
        carry_attribution_isolation_required=True,
    )
    row = {"close": 100.0, "funding_rate_zscore": 3.0}
    prop = ActionProposal(
        symbol="BTC-USD",
        route_id=RouteId.INTRADAY,
        direction=1,
        size_fraction=0.2,
        stop_distance_pct=0.02,
    )
    # Low trigger confidence → "directional neutrality" so carry may activate alongside a policy proposal
    dec = evaluate_carry_sleeve(
        row,
        _trig(0.05),
        _apex(),
        cfg,
        directional_proposal=prop,
    )
    assert dec.active is True
    assert dec.directional_blocked is True


def test_build_carry_proposal_route():
    from app.contracts.carry_sleeve import CarrySleeveDecision

    dec = CarrySleeveDecision(
        active=True,
        target_notional_usd=2000.0,
        funding_signal=0.8,
    )
    p = build_carry_proposal(
        "BTC-USD",
        dec,
        feature_row={"funding_rate_zscore": 1.0},
        max_per_symbol_usd=10_000.0,
    )
    assert p is not None
    assert p.route_id == RouteId.CARRY
    assert p.direction in (-1, 1)
