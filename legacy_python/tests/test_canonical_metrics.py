"""Tests for canonical monitoring metrics (FB-CAN-010)."""

from __future__ import annotations

from app.contracts.canonical_state import CanonicalStateOutput, DegradationLevel
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState
from datetime import UTC, datetime
from observability.canonical_metrics import record_canonical_post_tick


def _pkt() -> ForecastPacket:
    return ForecastPacket(
        timestamp=datetime.now(UTC),
        horizons=[1],
        q_low=[-0.01],
        q_med=[0.0],
        q_high=[0.01],
        interval_width=[0.02],
        regime_vector=[0.25, 0.25, 0.25, 0.25],
        confidence_score=0.7,
        ensemble_variance=[0.01],
        ood_score=0.15,
        forecast_diagnostics={
            "trigger": {
                "setup_valid": True,
                "pretrigger_valid": True,
                "trigger_valid": True,
                "missed_move_flag": False,
            },
            "auction": {"selected_score": 0.4, "selected_symbol": "X", "records": []},
        },
    )


def test_record_canonical_post_tick_runs():
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
        canonical_size_multiplier=0.9,
        last_risk_sizing={"final_notional_usd": 5000.0},
        data_age_seconds=2.0,
    )
    record_canonical_post_tick(symbol="BTC-USD", regime=regime, risk=risk, forecast_packet=_pkt())
