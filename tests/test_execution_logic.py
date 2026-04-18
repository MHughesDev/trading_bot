"""Tests for APEX execution logic (FB-CAN-008)."""

from __future__ import annotations

from app.contracts.execution_guidance import ExecutionFeedback
from execution.execution_logic import (
    apply_execution_feedback,
    build_execution_context_from_decision,
    compute_execution_confidence,
    prepare_order_intent_for_execution,
    reconcile_partial_fill,
)

from app.config.settings import AppSettings
from app.contracts.forecast import ForecastOutput
from app.contracts.orders import OrderIntent, OrderSide, OrderType
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState
from decimal import Decimal


def test_execution_confidence_in_range():
    ctx = {
        "depth_quality": 0.8,
        "spread_quality": 0.7,
        "venue_quality": 0.9,
        "latency_quality": 0.75,
        "slippage_quality": 0.6,
    }
    assert 0.0 <= compute_execution_confidence(ctx) <= 1.0


def test_reconcile_partial_fill_branches():
    assert reconcile_partial_fill(
        intended_qty=1.0,
        fill_ratio=0.99,
        remaining_edge=0.01,
        min_remaining_fraction=0.02,
        minimum_tradeable_edge=0.001,
        execution_confidence_realized=0.5,
        low_execution_floor=0.1,
    ) == "done"
    assert (
        reconcile_partial_fill(
            intended_qty=1.0,
            fill_ratio=0.4,
            remaining_edge=0.0001,
            min_remaining_fraction=0.01,
            minimum_tradeable_edge=0.002,
            execution_confidence_realized=0.5,
            low_execution_floor=0.1,
        )
        == "abandon"
    )


def test_feedback_updates_trust():
    b = apply_execution_feedback(
        "BTC-USD",
        ExecutionFeedback(realized_slippage_bps=80.0, venue_quality_score=0.5),
        state={},
    )
    assert "execution_trust" in b


def test_prepare_with_context_not_suppressed():
    settings = AppSettings(allow_unsigned_execution=True)
    intent = OrderIntent(
        symbol="BTC-USD",
        side=OrderSide.BUY,
        quantity=Decimal("0.01"),
        order_type=OrderType.MARKET,
        metadata={
            "route_id": "INTRADAY",
            "execution_context": {"spread_bps": 4.0, "heat_score": 0.2, "expected_edge": 0.02},
        },
    )
    out = prepare_order_intent_for_execution(intent, settings)
    assert out is not None
    assert "execution_guidance" in (out.metadata or {})


def test_build_context_from_decision():
    regime = RegimeOutput(
        state_index=0,
        semantic=SemanticRegime.BULL,
        probabilities=[0.5, 0.2, 0.2, 0.1],
        confidence=0.5,
    )
    fc = ForecastOutput(
        returns_1=0.01,
        returns_3=0.0,
        returns_5=0.0,
        returns_15=0.0,
        volatility=0.02,
        uncertainty=0.1,
    )
    ctx = build_execution_context_from_decision(
        spread_bps=6.0,
        feature_row={"close": 50000.0, "atr_14": 100.0, "return_1": 0.001, "volume": 1e6},
        regime=regime,
        forecast=fc,
        risk=RiskState(),
        mid_price=50000.0,
        forecast_packet=None,
    )
    assert "spread_bps" in ctx
    assert ctx["spread_bps"] == 6.0
