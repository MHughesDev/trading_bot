"""decision_trace forecast_packet_summary (FB-AUDIT-07)."""

from __future__ import annotations

from datetime import UTC, datetime

from app.contracts.forecast_packet import ForecastPacket
from decision_engine.audit import decision_trace
from app.contracts.decisions import RouteDecision, RouteId
from app.contracts.forecast import ForecastOutput
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState


def test_decision_trace_includes_packet_summary() -> None:
    pkt = ForecastPacket(
        timestamp=datetime.now(UTC),
        horizons=[1, 2],
        q_low=[-0.01, -0.02],
        q_med=[0.0, 0.01],
        q_high=[0.01, 0.02],
        interval_width=[0.02, 0.04],
        regime_vector=[0.25, 0.25, 0.25, 0.25],
        confidence_score=0.5,
        ensemble_variance=[0.0, 0.0],
        ood_score=0.1,
        forecast_diagnostics={"pipeline": "master_spec"},
        packet_schema_version=1,
        source_checkpoint_id="ck-1",
    )
    regime = RegimeOutput(
        state_index=0,
        semantic=SemanticRegime.BULL,
        probabilities=[0.25, 0.25, 0.25, 0.25],
        confidence=0.5,
    )
    fc = ForecastOutput(
        returns_1=0.0,
        returns_3=0.0,
        returns_5=0.0,
        returns_15=0.0,
        volatility=0.01,
        uncertainty=1.0,
    )
    route = RouteDecision(route_id=RouteId.INTRADAY, confidence=1.0, ranking=[RouteId.INTRADAY])
    tr = decision_trace(
        symbol="BTC-USD",
        regime=regime,
        forecast=fc,
        route=route,
        proposal=None,
        risk=RiskState(),
        trade_allowed=False,
        forecast_packet=pkt,
    )
    s = tr["forecast_packet_summary"]
    assert s is not None
    assert s["packet_schema_version"] == 1
    assert s["source_checkpoint_id"] == "ck-1"
    assert s["pipeline"] == "master_spec"
    assert s["q_med_head"] == [0.0, 0.01]


def test_decision_trace_packet_summary_none_when_omitted() -> None:
    regime = RegimeOutput(
        state_index=0,
        semantic=SemanticRegime.BULL,
        probabilities=[0.25, 0.25, 0.25, 0.25],
        confidence=0.5,
    )
    fc = ForecastOutput(
        returns_1=0.0,
        returns_3=0.0,
        returns_5=0.0,
        returns_15=0.0,
        volatility=0.01,
        uncertainty=1.0,
    )
    route = RouteDecision(route_id=RouteId.INTRADAY, confidence=1.0, ranking=[RouteId.INTRADAY])
    tr = decision_trace(
        symbol="BTC-USD",
        regime=regime,
        forecast=fc,
        route=route,
        proposal=None,
        risk=RiskState(),
        trade_allowed=False,
    )
    assert tr["forecast_packet_summary"] is None
