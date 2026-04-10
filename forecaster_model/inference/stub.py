"""Stub forecaster → `ForecastPacket` until full VSN/xLSTM stack ships (FB-FR-*)."""

from __future__ import annotations

from datetime import datetime

import numpy as np

from app.contracts.forecast_packet import ForecastPacket
from forecaster_model.config import ForecasterConfig
from forecaster_model.inference.build_from_ohlc import build_forecast_packet_methodology


def ohlc_arrays_from_feature_row(
    feature_row: dict[str, float],
    *,
    history_len: int = 64,
) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    """
    Build constant synthetic OHLCV bars from the latest feature row (for stub packet in pipeline).

    When only `close` (and optional `volume`) are present, repeats levels to fill history.
    """
    c0 = float(feature_row.get("close", 1.0))
    vol = float(feature_row.get("volume", 1e6))
    n = max(8, int(history_len))
    close = np.full(n, c0, dtype=np.float64)
    o = close * 0.9999
    h = close * 1.0001
    lo = close * 0.9998
    v = np.full(n, vol, dtype=np.float64)
    return o, h, lo, close, v


def build_forecast_packet_stub(
    open_: np.ndarray,
    high: np.ndarray,
    low: np.ndarray,
    close: np.ndarray,
    volume: np.ndarray,
    *,
    cfg: ForecasterConfig | None = None,
    now: datetime | None = None,
) -> ForecastPacket:
    """
    Delegates to `build_forecast_packet_methodology` (VSN → CNN → multi-res RNN → fusion → quantiles).

    Kept name for backward compatibility with callers.
    """
    return build_forecast_packet_methodology(
        open_, high, low, close, volume, cfg=cfg, now=now, seed=42
    )
