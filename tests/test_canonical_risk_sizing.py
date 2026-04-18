"""Tests for canonical risk sizing (FB-CAN-007)."""

from __future__ import annotations

from decimal import Decimal

from app.config.settings import AppSettings
from app.contracts.canonical_state import DegradationLevel
from app.contracts.decisions import ActionProposal, RouteId
from app.contracts.risk import RiskState
from datetime import UTC, datetime

import pytest

from decision_engine.state_engine import composite_degradation_size_multiplier, merge_canonical_into_risk
from risk_engine.canonical_sizing import (
    asymmetry_boost,
    classify_liquidation_mode,
    compute_canonical_notional,
    edge_budget_multiplier,
    inertia_multiplier,
)
from risk_engine.engine import RiskEngine


def test_edge_budget_reduces_when_hot():
    m = edge_budget_multiplier(heat=0.9, exposure_frac=0.5, symbol_exposure_frac=0.4)
    assert m < 1.0


def test_asymmetry_boost_capped_at_1_2():
    m, _ = asymmetry_boost(
        asymmetry=0.9,
        trigger_confidence=0.5,
        execution_confidence=0.5,
        heat=0.3,
        reflexivity=0.3,
    )
    assert m <= 1.2


def test_inertia_reduces_flip():
    m_same, _ = inertia_multiplier(
        direction=1,
        position_signed_qty=Decimal("0.1"),
        mid_price=100.0,
        equity_usd=100_000.0,
    )
    m_flip, reason = inertia_multiplier(
        direction=-1,
        position_signed_qty=Decimal("0.1"),
        mid_price=100.0,
        equity_usd=100_000.0,
    )
    assert m_same == 1.0
    assert m_flip < 1.0
    assert reason == "position_inertia_flip"


def test_compute_canonical_notional_positive():
    settings = AppSettings()
    prop = ActionProposal(
        symbol="X",
        route_id=RouteId.INTRADAY,
        direction=1,
        size_fraction=0.5,
        stop_distance_pct=0.02,
    )
    risk = RiskState(
        canonical_degradation=DegradationLevel.NORMAL,
        canonical_size_multiplier=1.0,
        risk_asymmetry_score=0.4,
        risk_trigger_confidence=0.4,
        risk_execution_confidence=0.8,
        risk_heat_score=0.2,
        risk_reflexivity_score=0.2,
        risk_liquidation_mode="neutral",
    )
    out = compute_canonical_notional(
        prop,
        risk,
        settings,
        mid_price=50_000.0,
        spread_bps=5.0,
        position_signed_qty=None,
        current_total_exposure_usd=0.0,
        portfolio_equity_usd=100_000.0,
    )
    assert out.final_notional_usd > 0
    assert out.diagnostics.final_notional_usd == out.final_notional_usd


def test_risk_engine_uses_canonical_notional():
    settings = AppSettings()
    eng = RiskEngine(settings)
    prop = ActionProposal(
        symbol="BTC-USD",
        route_id=RouteId.INTRADAY,
        direction=1,
        size_fraction=1.0,
        stop_distance_pct=0.02,
    )
    risk = RiskState(
        risk_asymmetry_score=0.3,
        risk_trigger_confidence=0.3,
        risk_execution_confidence=0.9,
        risk_heat_score=0.1,
        risk_reflexivity_score=0.1,
        risk_liquidation_mode="neutral",
    )
    trade, risk_out = eng.evaluate(
        "BTC-USD",
        prop,
        risk,
        mid_price=50_000.0,
        spread_bps=2.0,
        data_timestamp=datetime.now(UTC),
        current_total_exposure_usd=0.0,
        portfolio_equity_usd=100_000.0,
    )
    assert trade is not None
    assert risk_out.last_risk_sizing is not None
    assert "final_notional_usd" in risk_out.last_risk_sizing


def test_liquidation_mode_classify():
    assert classify_liquidation_mode(
        trigger_confidence=0.5,
        heat=0.3,
        asymmetry=0.5,
        atr_over_close=0.002,
        degradation=DegradationLevel.NORMAL,
    ) in ("offense", "neutral", "defense")


def test_composite_degradation_reduces_with_transition_and_novelty():
    from app.contracts.canonical_state import CanonicalStateOutput

    apex = CanonicalStateOutput(
        regime_probabilities=[0.2, 0.2, 0.2, 0.2, 0.2],
        regime_confidence=0.2,
        transition_probability=0.9,
        novelty=0.9,
        heat_score=0.3,
        reflexivity_score=0.3,
        degradation=DegradationLevel.NORMAL,
    )
    s = AppSettings()
    m, terms = composite_degradation_size_multiplier(DegradationLevel.NORMAL, apex, s)
    assert m < 1.0
    assert terms["transition_multiplier"] < 1.0
    assert terms["novelty_multiplier"] < 1.0


def test_fb_can_045_merge_then_sizing_diag_has_layer_fields():
    from app.contracts.canonical_state import CanonicalStateOutput

    apex = CanonicalStateOutput(
        regime_probabilities=[0.2, 0.2, 0.2, 0.2, 0.2],
        regime_confidence=0.2,
        transition_probability=0.5,
        novelty=0.4,
        heat_score=0.3,
        reflexivity_score=0.3,
        degradation=DegradationLevel.REDUCED,
    )
    settings = AppSettings()
    r0 = RiskState()
    r1 = merge_canonical_into_risk(r0, apex, settings=settings)
    prop = ActionProposal(
        symbol="X",
        route_id=RouteId.INTRADAY,
        direction=1,
        size_fraction=0.5,
        stop_distance_pct=0.02,
    )
    r1 = r1.model_copy(
        update={
            "risk_asymmetry_score": 0.4,
            "risk_trigger_confidence": 0.4,
            "risk_execution_confidence": 0.8,
            "risk_heat_score": 0.2,
            "risk_reflexivity_score": 0.2,
            "risk_liquidation_mode": "neutral",
        }
    )
    out = compute_canonical_notional(
        prop,
        r1,
        settings,
        mid_price=50_000.0,
        spread_bps=5.0,
        position_signed_qty=None,
        current_total_exposure_usd=0.0,
        portfolio_equity_usd=100_000.0,
    )
    d = out.diagnostics.model_dump()
    assert d["composite_degradation_multiplier"] == pytest.approx(float(r1.canonical_size_multiplier))
    prod = d["degradation_base_multiplier"] * d["transition_multiplier"] * d["novelty_multiplier"]
    assert prod == pytest.approx(d["composite_degradation_multiplier"], abs=1e-9)
    assert "edge_budget_multiplier" in d
    assert "config_snapshot" in d
