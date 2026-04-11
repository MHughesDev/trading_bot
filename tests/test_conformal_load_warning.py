"""IL-107: WARNING when conformal path set but load fails."""

from __future__ import annotations

import logging

import numpy as np
import pytest

from forecaster_model.config import ForecasterConfig
from forecaster_model.inference.build_from_ohlc import build_forecast_packet_methodology


def test_conformal_missing_path_logs_warning(caplog: pytest.LogCaptureFixture) -> None:
    rng = np.random.default_rng(0)
    n = 128
    c = 100 + np.cumsum(rng.normal(0, 0.2, size=n))
    o = np.roll(c, 1)
    o[0] = c[0]
    h = np.maximum(o, c) + 0.01
    lo = np.minimum(o, c) - 0.01
    v = rng.random(n) * 1e6
    cfg = ForecasterConfig(history_length=64, forecast_horizon=4, calibration_enabled=True)
    caplog.set_level(logging.WARNING)
    build_forecast_packet_methodology(
        o,
        h,
        lo,
        c,
        v,
        cfg=cfg,
        conformal_state_path="/nonexistent/path/conformal.json",
    )
    assert any("conformal state not loaded" in r.message for r in caplog.records)
