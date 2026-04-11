"""Conformal persistence and inference wiring (FB-FR-PG3)."""

from __future__ import annotations

import tempfile
from pathlib import Path

import numpy as np

from forecaster_model.calibration.conformal import (
    MultiHorizonConformal,
    SlidingConformalCalibrator,
    load_conformal_state,
    save_conformal_state,
)
from forecaster_model.config import ForecasterConfig
from forecaster_model.inference.build_from_ohlc import build_forecast_packet_methodology


def test_multi_horizon_roundtrip_json() -> None:
    m = MultiHorizonConformal.create(4, alpha=0.1, window_size=50)
    m.update_horizon(0, 0.0, -0.05, 0.05)
    with tempfile.TemporaryDirectory() as td:
        p = Path(td) / "conf.json"
        save_conformal_state(p, m)
        m2 = load_conformal_state(p)
        assert len(m2) == 4
        lo, md, hi = m2.apply_to_quantiles([-0.1], [0.0], [0.1])
        assert lo[0] <= -0.1
        assert hi[0] >= 0.1
        assert md[0] == 0.0


def test_sliding_calibrator_dict_roundtrip() -> None:
    c = SlidingConformalCalibrator(0.1, 20)
    c.update(0.0, -0.1, 0.1)
    raw = c.to_dict()
    c2 = SlidingConformalCalibrator.from_dict(raw)
    lo, hi = c2.calibrate(-0.05, 0.05)
    assert lo <= -0.05 <= hi


def test_build_packet_applies_conformal_when_enabled() -> None:
    rng = np.random.default_rng(1)
    n = 128
    c = 100 + np.cumsum(rng.normal(0, 0.2, size=n))
    o = np.roll(c, 1)
    o[0] = c[0]
    h = np.maximum(o, c) + 0.01
    lo = np.minimum(o, c) - 0.01
    v = rng.random(n) * 1e6
    cfg = ForecasterConfig(history_length=64, forecast_horizon=4, calibration_enabled=True)
    bundle = MultiHorizonConformal.create(4, alpha=0.1, window_size=100)
    pkt = build_forecast_packet_methodology(o, h, lo, c, v, cfg=cfg, conformal_bundle=bundle)
    assert pkt.forecast_diagnostics.get("conformal_applied") is True
