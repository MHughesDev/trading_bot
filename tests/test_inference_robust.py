from datetime import UTC, datetime

from app.contracts.forecast_packet import ForecastPacket
from forecaster_model.inference.robust import safe_build_forecast_packet


def _good_pkt():
    return ForecastPacket(
        timestamp=datetime.now(UTC),
        horizons=[1],
        q_low=[0.0],
        q_med=[0.0],
        q_high=[0.0],
        interval_width=[0.0],
        regime_vector=[0.25, 0.25, 0.25, 0.25],
        confidence_score=1.0,
        ensemble_variance=[0.0],
        ood_score=0.0,
    )


def test_safe_build_ok():
    pkt, reasons = safe_build_forecast_packet(lambda: _good_pkt())
    assert pkt is not None
    assert not reasons


def test_safe_build_raises():
    def boom():
        raise RuntimeError("simulated")

    pkt, reasons = safe_build_forecast_packet(boom)
    assert pkt is None
    assert any("build_error" in r for r in reasons)
