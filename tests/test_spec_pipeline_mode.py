"""decision_pipeline_mode=spec_policy uses PolicySystem path."""

from __future__ import annotations

import logging

from app.config.settings import AppSettings
from app.contracts.risk import RiskState
from decision_engine.pipeline import DecisionPipeline


def test_default_settings_use_spec_policy():
    assert AppSettings().decision_pipeline_mode == "spec_policy"


def test_legacy_mode_unchanged():
    s = AppSettings(decision_pipeline_mode="legacy")
    pipe = DecisionPipeline(settings=s)
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = 50_000.0
    feats["volume"] = 1e6
    risk = RiskState()
    regime, fc, route, proposal = pipe.step(
        "BTC-USD",
        feats,
        5.0,
        risk,
        mid_price=50_000.0,
        portfolio_equity_usd=100_000.0,
        position_signed_qty=None,
    )
    assert proposal is not None or route.route_id is not None


def test_spec_policy_stub_path_warning_logged_once(caplog):
    DecisionPipeline._stub_spec_path_logged = False
    caplog.set_level(logging.WARNING)
    s = AppSettings(decision_pipeline_mode="spec_policy")
    _ = DecisionPipeline(settings=s)
    msgs = [r.message for r in caplog.records if r.levelno >= logging.WARNING]
    assert any("build_forecast_packet_stub" in m and "HeuristicTargetPolicy" in m for m in msgs)
    caplog.clear()
    _ = DecisionPipeline(settings=s)
    assert not any("build_forecast_packet_stub" in r.message for r in caplog.records)


def test_spec_policy_mode_returns_proposal_or_none():
    s = AppSettings(decision_pipeline_mode="spec_policy")
    pipe = DecisionPipeline(settings=s)
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = 50_000.0
    feats["volume"] = 1e6
    risk = RiskState()
    regime, fc, route, proposal = pipe.step(
        "BTC-USD",
        feats,
        5.0,
        risk,
        mid_price=50_000.0,
        portfolio_equity_usd=100_000.0,
        position_signed_qty=None,
    )
    assert pipe.last_forecast_packet is not None
    assert pipe.last_forecast_packet.forecast_diagnostics.get("pipeline_mode") == "spec_policy"
    _ = regime, fc, route, proposal
