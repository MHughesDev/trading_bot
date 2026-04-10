"""Forecaster methodology path (VSN-CNN-RNN-fusion-quantile)."""

from __future__ import annotations

import numpy as np

from forecaster_model.config import ForecasterConfig
from forecaster_model.inference.build_from_ohlc import build_forecast_packet_methodology


def test_methodology_packet_monotone_quantiles() -> None:
    rng = np.random.default_rng(0)
    n = 128
    c = 100 + np.cumsum(rng.normal(0, 0.2, size=n))
    o = np.roll(c, 1)
    o[0] = c[0]
    h = np.maximum(o, c) + 0.01
    lo = np.minimum(o, c) - 0.01
    v = rng.random(n) * 1e6
    pkt = build_forecast_packet_methodology(o, h, lo, c, v, cfg=ForecasterConfig(history_length=64, forecast_horizon=4))
    for i in range(len(pkt.q_med)):
        assert pkt.q_low[i] <= pkt.q_med[i] <= pkt.q_high[i]


def test_sliding_conformal() -> None:
    from forecaster_model.calibration.conformal import SlidingConformalCalibrator

    c = SlidingConformalCalibrator(alpha=0.1, window_size=100)
    c.update(0.0, -0.1, 0.1)
    lo, hi = c.calibrate(-0.05, 0.05)
    assert lo <= -0.05 <= hi
