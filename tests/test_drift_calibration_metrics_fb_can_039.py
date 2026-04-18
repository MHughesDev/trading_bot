"""FB-CAN-039 drift / calibration Prometheus metrics."""

from __future__ import annotations

from datetime import UTC, datetime

from app.contracts.forecast_packet import ForecastPacket
from app.contracts.risk import RiskState
from observability.drift_calibration_metrics import (
    record_calibration_and_drift_from_tick,
    refresh_shadow_divergence_gauges_from_store,
)


def _pkt() -> ForecastPacket:
    return ForecastPacket(
        timestamp=datetime.now(UTC),
        horizons=[1],
        q_low=[-0.01],
        q_med=[0.0],
        q_high=[0.01],
        interval_width=[0.02],
        regime_vector=[0.25, 0.25, 0.25, 0.25],
        confidence_score=0.6,
        ensemble_variance=[0.01],
        ood_score=0.1,
        forecast_diagnostics={"conformal_applied": True},
    )


def test_record_calibration_trade_intent_observes_edge_metrics():
    risk = RiskState(
        last_decision_record={
            "outcome": "trade_intent",
            "forecast_summary": {"route_confidence": 0.5},
            "trade_intent": {
                "decision_confidence": 0.8,
                "trigger_confidence": 0.5,
                "execution_confidence": 0.2,
            },
        }
    )
    record_calibration_and_drift_from_tick(
        symbol="BTC-USD",
        risk=risk,
        forecast_packet=_pkt(),
        feature_row={"canonical_exec_quality_penalty": 0.1},
    )


def test_refresh_shadow_gauges_from_store():
    refresh_shadow_divergence_gauges_from_store(
        {
            "last_report": {
                "bars_compared": 100,
                "within_thresholds": True,
                "rollback_recommended": False,
                "rates": {"trigger_divergence": 0.02},
            }
        }
    )
