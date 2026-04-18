"""FB-CAN-076: edge-budget proxy metrics and escalation counters."""

from __future__ import annotations

from datetime import UTC, datetime
from unittest.mock import MagicMock, patch

from app.config.settings import AppSettings
from app.contracts.canonical_state import CanonicalStateOutput, DegradationLevel
from app.contracts.decisions import ActionProposal, RouteId
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState
import observability.canonical_metrics as cm
from risk_engine.canonical_sizing import compute_canonical_notional


def test_compute_canonical_notional_edge_budget_stress_fields():
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
        risk_heat_score=0.95,
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
        current_total_exposure_usd=95_000.0,
        portfolio_equity_usd=100_000.0,
    )
    d = out.diagnostics
    assert d.edge_budget_multiplier < 0.999
    assert abs(d.edge_budget_headroom - d.edge_budget_multiplier) < 1e-9
    assert abs(d.edge_budget_stress - (1.0 - d.edge_budget_multiplier)) < 1e-9


def test_edge_budget_escalation_increments_counters():
    settings = AppSettings()
    pkt = ForecastPacket(
        timestamp=datetime.now(UTC),
        horizons=[1],
        q_low=[0.0],
        q_med=[0.0],
        q_high=[0.0],
        interval_width=[0.01],
        regime_vector=[0.25, 0.25, 0.25, 0.25],
        confidence_score=0.5,
        ensemble_variance=[0.01],
        ood_score=0.1,
        forecast_diagnostics={
            "auction": {
                "selected_symbol": None,
                "records": [{"penalties": {"B": 0.6}}],
            }
        },
    )
    risk = RiskState(
        last_risk_sizing={
            "edge_budget_stress": 0.4,
            "edge_budget_headroom": 0.6,
        },
        last_decision_record={
            "outcome": "trade_intent",
            "trade_intent": {
                "decision_confidence": 0.9,
                "trigger_confidence": 0.9,
                "execution_confidence": 0.2,
            },
        },
    )
    inc_mock = MagicMock()
    with patch.object(cm, "CANONICAL_EDGE_BUDGET_ESCALATION") as ctr:
        ctr.labels.return_value.inc = inc_mock
        cm._edge_budget_escalation_metrics(
            symbol="BTC-USD",
            risk=risk,
            forecast_packet=pkt,
            settings=settings,
        )
    assert inc_mock.call_count >= 1


def test_record_post_tick_observes_edge_budget_histograms():
    ts = datetime.now(UTC)
    pkt = ForecastPacket(
        timestamp=ts,
        horizons=[1],
        q_low=[0.0],
        q_med=[0.0],
        q_high=[0.0],
        interval_width=[0.01],
        regime_vector=[0.25, 0.25, 0.25, 0.25],
        confidence_score=0.5,
        ensemble_variance=[0.01],
        ood_score=0.1,
        forecast_diagnostics={
            "auction": {
                "selected_score": 0.1,
                "selected_symbol": "X",
                "records": [{"penalties": {"B": 0.4}}],
            }
        },
    )
    apex = CanonicalStateOutput(
        regime_probabilities=[0.2, 0.2, 0.2, 0.2, 0.2],
        regime_confidence=0.5,
        transition_probability=0.2,
        novelty=0.1,
        heat_score=0.3,
        reflexivity_score=0.2,
        degradation=DegradationLevel.NORMAL,
    )
    regime = RegimeOutput(
        state_index=0,
        semantic=SemanticRegime.BULL,
        probabilities=[1.0, 0, 0, 0],
        confidence=0.8,
        apex=apex,
    )
    risk = RiskState(
        last_risk_sizing={
            "final_notional_usd": 1000.0,
            "edge_budget_headroom": 0.7,
            "edge_budget_multiplier": 0.7,
        }
    )
    observe_h = MagicMock()
    observe_b = MagicMock()
    with patch.object(cm, "CANONICAL_EDGE_BUDGET_HEADROOM") as h_h:
        with patch.object(cm, "CANONICAL_AUCTION_EDGE_PENALTY") as b_h:
            h_h.labels.return_value.observe = observe_h
            b_h.labels.return_value.observe = observe_b
            cm.record_canonical_post_tick(
                symbol="BTC-USD",
                regime=regime,
                risk=risk,
                forecast_packet=pkt,
                settings=AppSettings(),
            )
    observe_h.assert_called()
    observe_b.assert_called()
