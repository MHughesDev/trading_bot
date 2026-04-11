"""FB-AUDIT-05: fixed-seed OHLC + fixed anchor time → `ForecastPacket` golden snapshot."""

from __future__ import annotations

import json
from datetime import datetime
from pathlib import Path

import numpy as np
import pytest

from forecaster_model.config import ForecasterConfig
from forecaster_model.inference.build_from_ohlc import build_forecast_packet_methodology

_FIXTURE = Path(__file__).resolve().parent / "fixtures" / "forecaster_golden_packet.json"


def _synthetic_ohlc() -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    rng = np.random.default_rng(12345)
    n = 128
    c = 100 + np.cumsum(rng.normal(0, 0.1, size=n))
    o = np.roll(c, 1)
    o[0] = c[0]
    h = np.maximum(o, c) + 0.01
    lo = np.minimum(o, c) - 0.01
    v = rng.random(n) * 1e6
    return o, h, lo, c, v


def test_forecaster_packet_matches_golden_snapshot() -> None:
    raw = json.loads(_FIXTURE.read_text(encoding="utf-8"))
    assert raw["schema_version"] >= 2
    anchor = datetime.fromisoformat(raw["anchor_iso"])
    o, h, lo, c, v = _synthetic_ohlc()
    cfg = ForecasterConfig(
        history_length=int(raw["history_length"]),
        forecast_horizon=int(raw["forecast_horizon"]),
    )
    pkt = build_forecast_packet_methodology(
        o,
        h,
        lo,
        c,
        v,
        cfg=cfg,
        seed=int(raw["build_seed"]),
        now=anchor,
    )
    exp_med = raw["q_med"]
    assert len(pkt.q_med) == len(exp_med)
    np.testing.assert_allclose(pkt.q_med, exp_med, rtol=1e-12, atol=1e-12)
    assert float(pkt.forecast_diagnostics["vsn_gate_mean"]) == pytest.approx(float(raw["vsn_gate_mean"]))
    np.testing.assert_allclose(
        pkt.forecast_diagnostics["fusion_alpha"],
        raw["fusion_alpha"],
        rtol=1e-12,
        atol=1e-12,
    )
