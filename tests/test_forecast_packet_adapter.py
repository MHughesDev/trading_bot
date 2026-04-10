"""ForecastPacket → ForecastOutput adapter (FB-FR-PG1)."""

from __future__ import annotations

from datetime import UTC, datetime

from app.contracts.forecast_packet import ForecastPacket
from decision_engine.forecast_packet_adapter import forecast_packet_to_forecast_output


def test_adapter_maps_medians_to_horizons() -> None:
    pkt = ForecastPacket(
        timestamp=datetime.now(UTC),
        horizons=[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        q_low=[0.0] * 15,
        q_med=[float(i) * 1e-4 for i in range(1, 16)],
        q_high=[0.0] * 15,
        interval_width=[0.01] * 15,
        regime_vector=[0.25, 0.25, 0.25, 0.25],
        confidence_score=1.0,
        ensemble_variance=[0.0] * 15,
        ood_score=0.0,
    )
    out = forecast_packet_to_forecast_output(pkt)
    assert out.returns_1 == pkt.q_med[0]
    assert out.returns_3 == pkt.q_med[2]
    assert out.returns_5 == pkt.q_med[4]
    assert out.returns_15 == pkt.q_med[14]
    assert out.volatility >= 0.0
