"""Stub forecaster → `ForecastPacket` until full VSN/xLSTM stack ships (FB-FR-*)."""

from __future__ import annotations

from datetime import UTC, datetime

import numpy as np

from app.contracts.forecast_packet import ForecastPacket
from forecaster_model.config import ForecasterConfig
from forecaster_model.features.ohlc import build_observed_feature_matrix, log_returns
from forecaster_model.regime.soft import soft_regime_from_returns


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
    Heuristic quantile forecasts from recent momentum/vol (no learned weights).

    Integrates with the same `ForecastPacket` contract as the future neural forecaster.
    """
    cfg = cfg or ForecasterConfig()
    c = np.asarray(close, dtype=np.float64).ravel()
    if len(c) < 8:
        h = cfg.forecast_horizon
        z = [0.0] * h
        return ForecastPacket(
            timestamp=now or datetime.now(UTC),
            horizons=list(range(1, h + 1)),
            q_low=z.copy(),
            q_med=z.copy(),
            q_high=z.copy(),
            interval_width=z.copy(),
            regime_vector=[0.25, 0.25, 0.25, 0.25],
            confidence_score=0.0,
            ensemble_variance=z.copy(),
            ood_score=1.0,
            forecast_diagnostics={"stub": True, "reason": "insufficient_history"},
        )

    lr = log_returns(c)
    L = min(cfg.history_length, len(c))
    sl = slice(-L, None)
    x_obs = build_observed_feature_matrix(
        open_[sl], high[sl], low[sl], close[sl], volume[sl], windows=cfg.feature_windows
    )
    # Last row summary → scalar edge
    edge = float(np.mean(x_obs[-1, : min(8, x_obs.shape[1])]))
    vol = float(np.std(lr[-32:])) + 1e-12
    H = cfg.forecast_horizon
    q_med = [edge / (1.0 + j * 0.1) for j in range(H)]
    width = [1.645 * vol * (1.0 + 0.05 * j) for j in range(H)]
    q_low = [m - w for m, w in zip(q_med, width, strict=True)]
    q_high = [m + w for m, w in zip(q_med, width, strict=True)]
    iv = [qh - ql for qh, ql in zip(q_high, q_low, strict=True)]
    regime = soft_regime_from_returns(lr, num_regimes=cfg.num_regime_dims).tolist()
    conf = float(abs(edge) / (vol + 1e-9))
    ens_var = [1e-8 * (j + 1) for j in range(H)]

    return ForecastPacket(
        timestamp=now or datetime.now(UTC),
        horizons=list(range(1, H + 1)),
        q_low=q_low,
        q_med=q_med,
        q_high=q_high,
        interval_width=iv,
        regime_vector=regime,
        confidence_score=conf,
        ensemble_variance=ens_var,
        ood_score=0.0 if vol < 0.05 else min(1.0, vol),
        forecast_diagnostics={
            "stub": True,
            "x_obs_shape": list(x_obs.shape),
            "feature_dim": int(x_obs.shape[1]),
        },
    )
