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


def test_routing_uses_ridge_by_default() -> None:
    s = AppSettings(decision_forecast_routing_source="ridge")
    pipe = DecisionPipeline(settings=s)
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = 50_000.0
    feats["volume"] = 1e6
    risk = RiskState()
    _, fc_ridge_only, _, _ = pipe.step("BTC-USD", feats, spread_bps=5.0, risk=risk)
    s2 = AppSettings(
        decision_forecast_routing_source="ridge",
        decision_forecast_packet_enabled=True,
    )
    pipe2 = DecisionPipeline(settings=s2)
    _, fc_with_packet, _, _ = pipe2.step("BTC-USD", feats, spread_bps=5.0, risk=risk)
    assert fc_ridge_only.returns_5 == fc_with_packet.returns_5


def test_routing_packet_derives_forecast_output() -> None:
    s = AppSettings(decision_forecast_routing_source="packet")
    pipe = DecisionPipeline(settings=s)
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = 50_000.0
    feats["volume"] = 1e6
    risk = RiskState()
    regime_out, fc, route, action = pipe.step("BTC-USD", feats, spread_bps=5.0, risk=risk)
    assert pipe.last_forecast_packet is not None
    assert pipe.last_forecast_packet.forecast_diagnostics.get("routing_source") == "packet"
    # Packet-driven fc must match adapter output
    from decision_engine.forecast_packet_adapter import forecast_packet_to_forecast_output

    assert fc.returns_5 == forecast_packet_to_forecast_output(pipe.last_forecast_packet).returns_5
    _ = regime_out, route, action
