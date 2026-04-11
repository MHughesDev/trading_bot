"""Master pipeline: forecaster (xLSTM stack) → ForecastPacket → PolicySystem."""

from __future__ import annotations

from app.config.settings import AppSettings
from app.contracts.risk import RiskState
from decision_engine.pipeline import DecisionPipeline


def test_default_settings_have_forecaster_fields():
    s = AppSettings()
    assert s.models_forecaster_checkpoint_id is None
    assert s.models_forecaster_conformal_state_path is None


def test_master_pipeline_step_runs():
    pipe = DecisionPipeline(settings=AppSettings())
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
    assert pipe.last_forecast_packet.forecast_diagnostics.get("pipeline") == "master_spec"
    assert regime.semantic is not None
    assert fc.volatility >= 0.0
    assert route.route_id is not None
    _ = proposal
