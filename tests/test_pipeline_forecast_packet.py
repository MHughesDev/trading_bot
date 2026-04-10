"""DecisionPipeline optional ForecastPacket (FB-PL-P1)."""

from __future__ import annotations

from app.config.settings import AppSettings
from app.contracts.risk import RiskState
from decision_engine.pipeline import DecisionPipeline


def test_forecast_packet_disabled_by_default() -> None:
    pipe = DecisionPipeline(settings=AppSettings())
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = 50_000.0
    feats["volume"] = 1e6
    risk = RiskState()
    pipe.step("BTC-USD", feats, spread_bps=5.0, risk=risk)
    assert pipe.last_forecast_packet is None


def test_forecast_packet_when_enabled() -> None:
    s = AppSettings(decision_forecast_packet_enabled=True)
    pipe = DecisionPipeline(settings=s)
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = 50_000.0
    feats["volume"] = 1e6
    risk = RiskState()
    pipe.step("BTC-USD", feats, spread_bps=5.0, risk=risk)
    assert pipe.last_forecast_packet is not None
    assert len(pipe.last_forecast_packet.q_med) >= 1
