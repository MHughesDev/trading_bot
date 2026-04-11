import numpy as np

from app.contracts.forecast_packet import ForecastPacket
from datetime import UTC, datetime
from forecaster_model.models.ensemble import build_ensemble_forecast_packet, forward_ensemble_numpy


def test_build_ensemble_packet():
    ts = datetime.now(UTC)
    def pkt(seed: float):
        return ForecastPacket(
            timestamp=ts,
            horizons=[1, 2],
            q_low=[-0.1, -0.1],
            q_med=[seed, seed + 0.01],
            q_high=[0.1, 0.1],
            interval_width=[0.2, 0.2],
            regime_vector=[0.25, 0.25, 0.25, 0.25],
            confidence_score=1.0,
            ensemble_variance=[0.0, 0.0],
            ood_score=0.0,
        )
    out = build_ensemble_forecast_packet([pkt(0.0), pkt(0.2)])
    assert len(out.ensemble_variance) == 2
    assert out.forecast_diagnostics.get("ensemble_members") == 2


def test_forward_ensemble_numpy():
    rng = np.random.default_rng(0)
    x_obs = rng.normal(size=(32, 8))
    x_known = rng.normal(size=(4, 4))
    r_cur = np.ones(4) / 4
    y, d = forward_ensemble_numpy(x_obs, x_known, r_cur, num_members=3)
    assert y.shape[0] == 4
    assert d["num_members"] == 3
