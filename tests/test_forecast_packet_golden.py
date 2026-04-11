"""Golden OHLC slice → `ForecastPacket` diagnostics (FB-AUDIT-05): catches silent `ForecasterModel` drift."""

from __future__ import annotations

from datetime import UTC, datetime

import numpy as np
import pytest

from forecaster_model.config import ForecasterConfig
from forecaster_model.inference.build_from_ohlc import build_forecast_packet_methodology

_GOLDEN_ANCHOR = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)


def _golden_ohlc() -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    rng = np.random.default_rng(12345)
    n = 128
    c = 100 + np.cumsum(rng.normal(0, 0.2, size=n))
    o = np.roll(c, 1)
    o[0] = c[0]
    h = np.maximum(o, c) + 0.01
    lo = np.minimum(o, c) - 0.01
    v = rng.random(n) * 1e6
    return o, h, lo, c, v


def test_forecast_packet_golden_quantiles_and_diagnostics() -> None:
    o, h, lo, c, v = _golden_ohlc()
    cfg = ForecasterConfig(history_length=64, forecast_horizon=4)
    pkt = build_forecast_packet_methodology(
        o, h, lo, c, v, cfg=cfg, seed=42, now=_GOLDEN_ANCHOR
    )

    expected_q_low = [-0.9263822787804096, -0.9307496717614141, -0.9348505138514333, -0.9386399404072864]
    expected_q_med = [0.002467613706990643, 0.0005582540550563524, -0.0009054810761829872, -0.0019064997430771932]
    expected_q_high = [1.0512303100468505, 1.0520617133940255, 1.0526744253532447, 1.053060163155818]

    np.testing.assert_allclose(pkt.q_low, expected_q_low, rtol=0, atol=1e-12)
    np.testing.assert_allclose(pkt.q_med, expected_q_med, rtol=0, atol=1e-12)
    np.testing.assert_allclose(pkt.q_high, expected_q_high, rtol=0, atol=1e-12)

    assert pkt.forecast_diagnostics.get("methodology") == "numpy_reference"
    assert "vsn_gate_mean" in pkt.forecast_diagnostics


def test_models_torch_device_default(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("NM_MODELS_TORCH_DEVICE", raising=False)
    from app.config.settings import load_settings

    s = load_settings()
    assert s.models_torch_device == "auto"


@pytest.mark.parametrize("raw", ["auto", "cpu"])
def test_resolve_torch_device_cpu_paths(raw: str) -> None:
    pytest.importorskip("torch")
    from forecaster_model.training.device import resolve_torch_device

    out = resolve_torch_device(raw)
    assert out == "cpu"
