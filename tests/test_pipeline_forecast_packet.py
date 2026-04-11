"""DecisionPipeline always builds a master-spec ForecastPacket + PolicySystem path."""

from __future__ import annotations

import logging

import pytest

from app.config.settings import AppSettings
from app.contracts.risk import RiskState
from decision_engine.forecast_packet_adapter import forecast_packet_to_forecast_output
from decision_engine import pipeline as pipeline_mod
from decision_engine.pipeline import DecisionPipeline


def _features(close: float = 50_000.0) -> dict[str, float]:
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = close
    feats["volume"] = 1e6
    return feats


def test_serving_mode_logged_once(caplog: pytest.LogCaptureFixture) -> None:
    pipeline_mod._serving_mode_logged = False
    caplog.set_level(logging.INFO)
    pipe = DecisionPipeline(settings=AppSettings())
    risk = RiskState()
    pipe.step("BTC-USD", _features(), spread_bps=5.0, risk=risk, mid_price=50_000.0, portfolio_equity_usd=100_000.0)
    assert any(
        "decision pipeline serving mode" in r.message and "numpy_rng" in r.message and "heuristic" in r.message
        for r in caplog.records
    )


def test_pipeline_always_has_forecast_packet() -> None:
    pipe = DecisionPipeline(settings=AppSettings())
    risk = RiskState()
    pipe.step(
        "BTC-USD",
        _features(),
        spread_bps=5.0,
        risk=risk,
        mid_price=50_000.0,
        portfolio_equity_usd=100_000.0,
    )
    assert pipe.last_forecast_packet is not None
    pkt = pipe.last_forecast_packet
    assert len(pkt.q_med) >= 1
    assert pkt.forecast_diagnostics.get("pipeline") == "master_spec"
    assert pkt.packet_schema_version == 1


def test_forecast_output_derived_from_packet() -> None:
    pipe = DecisionPipeline(settings=AppSettings())
    risk = RiskState()
    _, fc, _, _ = pipe.step(
        "BTC-USD",
        _features(),
        spread_bps=5.0,
        risk=risk,
        mid_price=50_000.0,
        portfolio_equity_usd=100_000.0,
    )
    assert pipe.last_forecast_packet is not None
    assert fc.returns_5 == forecast_packet_to_forecast_output(pipe.last_forecast_packet).returns_5


def test_checkpoint_id_on_packet_when_set() -> None:
    pipe = DecisionPipeline(
        settings=AppSettings(models_forecaster_checkpoint_id="ckpt-test-001"),
    )
    risk = RiskState()
    pipe.step(
        "BTC-USD",
        _features(),
        spread_bps=5.0,
        risk=risk,
        mid_price=50_000.0,
        portfolio_equity_usd=100_000.0,
    )
    assert pipe.last_forecast_packet is not None
    assert pipe.last_forecast_packet.source_checkpoint_id == "ckpt-test-001"
