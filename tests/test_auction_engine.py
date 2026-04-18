"""Tests for opportunity auction (FB-CAN-006)."""

from __future__ import annotations

from datetime import UTC, datetime
from decimal import Decimal

from app.config.settings import AppSettings
from app.contracts.canonical_state import CanonicalStateOutput, DegradationLevel
from app.contracts.decisions import ActionProposal, RouteId
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.risk import RiskState
from app.contracts.trigger import TriggerOutput
from decision_engine.auction_engine import run_opportunity_auction


def _pkt() -> ForecastPacket:
    return ForecastPacket(
        timestamp=datetime.now(UTC),
        horizons=[1, 3, 5],
        q_low=[-0.02, -0.03, -0.04],
        q_med=[0.02, 0.0, 0.0],
        q_high=[0.04, 0.05, 0.06],
        interval_width=[0.06, 0.08, 0.1],
        regime_vector=[0.35, 0.25, 0.2, 0.2],
        confidence_score=0.7,
        ensemble_variance=[0.01, 0.02, 0.03],
        ood_score=0.1,
    )


def _apex() -> CanonicalStateOutput:
    return CanonicalStateOutput(
        regime_probabilities=[0.3, 0.25, 0.2, 0.15, 0.1],
        regime_confidence=0.5,
        transition_probability=0.2,
        novelty=0.1,
        heat_score=0.2,
        reflexivity_score=0.2,
        degradation=DegradationLevel.NORMAL,
    )


def _trigger_ok() -> TriggerOutput:
    return TriggerOutput(
        setup_valid=True,
        setup_score=0.5,
        pretrigger_valid=True,
        pretrigger_score=0.5,
        trigger_valid=True,
        trigger_type="composite_confirmed",
        trigger_strength=0.5,
        trigger_confidence=0.6,
        missed_move_flag=False,
        trigger_reason_codes=[],
    )


def test_auction_selects_directional_when_base_present():
    base = ActionProposal(
        symbol="BTC/USD",
        route_id=RouteId.INTRADAY,
        direction=1,
        size_fraction=0.25,
        stop_distance_pct=0.02,
    )
    settings = AppSettings()
    risk = RiskState()
    feat = {"close": 50_000.0, "atr_14": 100.0}
    prop, result = run_opportunity_auction(
        "BTC/USD",
        _pkt(),
        apex=_apex(),
        trigger=_trigger_ok(),
        app_risk=risk,
        spread_bps=5.0,
        feature_row=feat,
        settings=settings,
        portfolio_equity_usd=100_000.0,
        position_signed_qty=Decimal("0"),
        base_proposal=base,
    )
    assert prop is not None
    assert prop.direction in (1, -1)
    assert result.selected_direction == prop.direction
    assert any(r.eligible for r in result.records if r.direction != 0)


def test_no_trade_degradation_rejects_directional():
    base = ActionProposal(
        symbol="BTC/USD",
        route_id=RouteId.INTRADAY,
        direction=1,
        size_fraction=0.25,
        stop_distance_pct=0.02,
    )
    settings = AppSettings()
    risk = RiskState()
    apex = _apex().model_copy(update={"degradation": DegradationLevel.NO_TRADE})
    prop, result = run_opportunity_auction(
        "BTC/USD",
        _pkt(),
        apex=apex,
        trigger=_trigger_ok(),
        app_risk=risk,
        spread_bps=5.0,
        feature_row={"close": 1.0},
        settings=settings,
        portfolio_equity_usd=100_000.0,
        position_signed_qty=None,
        base_proposal=base,
    )
    assert prop is None
    assert result.selected_direction == 0


def test_missed_move_excludes_directional():
    base = ActionProposal(
        symbol="BTC/USD",
        route_id=RouteId.INTRADAY,
        direction=1,
        size_fraction=0.25,
        stop_distance_pct=0.02,
    )
    trig = TriggerOutput(
        setup_valid=True,
        setup_score=0.5,
        pretrigger_valid=True,
        pretrigger_score=0.5,
        trigger_valid=False,
        trigger_type="composite_confirmed",
        trigger_strength=0.5,
        trigger_confidence=0.6,
        missed_move_flag=True,
        trigger_reason_codes=["move_already_extended"],
    )
    prop, result = run_opportunity_auction(
        "BTC/USD",
        _pkt(),
        apex=_apex(),
        trigger=trig,
        app_risk=RiskState(),
        spread_bps=5.0,
        feature_row={"close": 50_000.0, "atr_14": 100.0},
        settings=AppSettings(),
        portfolio_equity_usd=100_000.0,
        position_signed_qty=Decimal("0"),
        base_proposal=base,
    )
    assert prop is None
    assert result.selected_direction == 0
    assert any("missed_move" in r.reasons for r in result.records if r.direction != 0)


def test_false_positive_memory_increases_auction_penalty():
    base = ActionProposal(
        symbol="BTC/USD",
        route_id=RouteId.INTRADAY,
        direction=1,
        size_fraction=0.25,
        stop_distance_pct=0.02,
    )
    risk0 = RiskState(trigger_false_positive_memory=0.0)
    risk1 = RiskState(trigger_false_positive_memory=0.85)
    feat = {"close": 50_000.0, "atr_14": 100.0}
    _, r0 = run_opportunity_auction(
        "BTC/USD",
        _pkt(),
        apex=_apex(),
        trigger=_trigger_ok(),
        app_risk=risk0,
        spread_bps=5.0,
        feature_row=feat,
        settings=AppSettings(),
        portfolio_equity_usd=100_000.0,
        position_signed_qty=Decimal("0"),
        base_proposal=base,
    )
    _, r1 = run_opportunity_auction(
        "BTC/USD",
        _pkt(),
        apex=_apex(),
        trigger=_trigger_ok(),
        app_risk=risk1,
        spread_bps=5.0,
        feature_row=feat,
        settings=AppSettings(),
        portfolio_equity_usd=100_000.0,
        position_signed_qty=Decimal("0"),
        base_proposal=base,
    )
    long0 = next(r for r in r0.records if r.direction == 1)
    long1 = next(r for r in r1.records if r.direction == 1)
    assert long1.penalties["P"] > long0.penalties["P"]


def test_ranking_stable_tiebreak():
    """Same inputs → same winner (deterministic)."""
    base = ActionProposal(
        symbol="ETH/USD",
        route_id=RouteId.INTRADAY,
        direction=1,
        size_fraction=0.2,
        stop_distance_pct=0.02,
    )
    settings = AppSettings()
    a1, r1 = run_opportunity_auction(
        "ETH/USD",
        _pkt(),
        apex=_apex(),
        trigger=_trigger_ok(),
        app_risk=RiskState(),
        spread_bps=5.0,
        feature_row={"close": 3000.0},
        settings=settings,
        portfolio_equity_usd=100_000.0,
        position_signed_qty=Decimal("0"),
        base_proposal=base,
    )
    a2, r2 = run_opportunity_auction(
        "ETH/USD",
        _pkt(),
        apex=_apex(),
        trigger=_trigger_ok(),
        app_risk=RiskState(),
        spread_bps=5.0,
        feature_row={"close": 3000.0},
        settings=settings,
        portfolio_equity_usd=100_000.0,
        position_signed_qty=Decimal("0"),
        base_proposal=base,
    )
    assert (a1.direction if a1 else None) == (a2.direction if a2 else None)
    assert r1.selected_score == r2.selected_score
