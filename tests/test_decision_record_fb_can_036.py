"""FB-CAN-036 canonical decision record and risk block codes."""

from __future__ import annotations

from datetime import UTC, datetime

import pytest
from app.config.settings import AppSettings
from app.contracts.decisions import ActionProposal, RouteDecision, RouteId
from app.contracts.forecast import ForecastOutput
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState, SystemMode
from decision_engine.decision_record import build_decision_record
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RISK_BLOCK_PAUSE_NEW_ENTRIES, RiskEngine


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


def test_build_decision_record_no_trade_has_codes():
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
    route = RouteDecision(route_id=RouteId.NO_TRADE, confidence=0.0, ranking=[RouteId.NO_TRADE])
    risk = RiskState(
        last_pipeline_no_trade_codes=["pipeline_no_trade_selected"],
        last_risk_block_codes=[],
    )
    pkt = _pkt()
    dr = build_decision_record(
        symbol="BTC-USD",
        data_timestamp=datetime.now(UTC),
        settings=AppSettings(),
        regime=regime,
        forecast=fc,
        route=route,
        proposal=None,
        risk=risk,
        forecast_packet=pkt,
        trade=None,
    )
    assert dr.no_trade is not None
    assert "pipeline_no_trade_selected" in dr.no_trade.no_trade_reason_codes
    assert dr.forecast_summary.get("route_confidence") == pytest.approx(0.0)


def test_risk_engine_sets_block_code_on_pause():
    eng = RiskEngine(AppSettings())
    prop = ActionProposal(
        symbol="BTC-USD",
        route_id=RouteId.INTRADAY,
        direction=1,
        size_fraction=0.1,
        stop_distance_pct=0.02,
    )
    _, risk = eng.evaluate(
        "BTC-USD",
        prop,
        RiskState(mode=SystemMode.PAUSE_NEW_ENTRIES),
        mid_price=50_000.0,
        spread_bps=5.0,
        data_timestamp=datetime.now(UTC),
    )
    assert RISK_BLOCK_PAUSE_NEW_ENTRIES in risk.last_risk_block_codes


def test_pipeline_attaches_decision_record_to_risk():
    pipe = DecisionPipeline(settings=AppSettings())
    risk = RiskState()
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = 50_000.0
    feats["volume"] = 1e6
    from decision_engine.run_step import run_decision_tick
    from risk_engine.engine import RiskEngine

    eng = RiskEngine(AppSettings())
    regime, fc, route, proposal, trade, risk_out = run_decision_tick(
        symbol="BTC-USD",
        feature_row=feats,
        spread_bps=5.0,
        risk_state=risk,
        pipeline=pipe,
        risk_engine=eng,
        mid_price=50_000.0,
        data_timestamp=datetime.now(UTC),
        portfolio_equity_usd=100_000.0,
        replay_deterministic=True,
    )
    assert risk_out.last_decision_record is not None
    assert risk_out.last_decision_record.get("schema_version") == 1
    assert risk_out.last_decision_record.get("instrument_id") == "BTC-USD"
    _ = regime, fc, route, proposal, trade
