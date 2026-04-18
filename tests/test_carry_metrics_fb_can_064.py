"""FB-CAN-064: carry sleeve Prometheus metrics from record_canonical_post_tick."""

from __future__ import annotations

from app.contracts.canonical_state import CanonicalStateOutput, DegradationLevel
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState
from observability.canonical_metrics import record_canonical_post_tick


def test_carry_metrics_from_carry_sleeve_dict():
    apex = CanonicalStateOutput(
        regime_probabilities=[0.2, 0.2, 0.2, 0.2, 0.2],
        regime_confidence=0.5,
        transition_probability=0.1,
        novelty=0.1,
        heat_score=0.2,
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
    cs = {
        "active": True,
        "funding_signal": 0.8,
        "trigger_confidence": 0.5,
        "decision_quality": 0.4,
        "target_notional_usd": 2500.0,
        "reason_codes": ["carry_active", "funding_ok"],
        "directional_blocked": True,
    }
    record_canonical_post_tick(
        symbol="BTC-USD",
        regime=regime,
        risk=RiskState(),
        forecast_packet=None,
        carry_sleeve=cs,
    )
