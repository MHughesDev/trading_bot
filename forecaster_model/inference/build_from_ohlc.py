"""Build `ForecastPacket` using spec methodology: normalize → regime → numpy reference forward."""

from __future__ import annotations

from datetime import UTC, datetime

import numpy as np

from app.contracts.forecast_packet import ForecastPacket
from forecaster_model.config import ForecasterConfig
from forecaster_model.features.normalization import rolling_zscore_causal
from forecaster_model.features.ohlc import build_observed_feature_matrix
from forecaster_model.features.time_future import known_future_features
from forecaster_model.models.numpy_reference import forward_numpy_reference
from forecaster_model.regime.soft import soft_regime_from_returns


def build_forecast_packet_methodology(
    open_: np.ndarray,
    high: np.ndarray,
    low: np.ndarray,
    close: np.ndarray,
    volume: np.ndarray,
    *,
    cfg: ForecasterConfig | None = None,
    now: datetime | None = None,
    seed: int = 42,
) -> ForecastPacket:
    """
    End-to-end reference path aligned with human forecaster spec (NumPy reference model).

    - Causal rolling z-score on x_obs
    - Soft regime from log returns
    - Known-future cyclical features
    - VSN → CNN → multi-res RNN → fusion → quantile decoder
    """
    cfg = cfg or ForecasterConfig()
    c = np.asarray(close, dtype=np.float64).ravel()
    if len(c) < max(16, cfg.history_length):
        return _empty_packet(cfg, now)

    L = min(cfg.history_length, len(c))
    sl = slice(-L, None)
    o, h, lo, cl, vo = (
        open_[sl],
        high[sl],
        low[sl],
        close[sl],
        volume[sl],
    )
    x_obs = build_observed_feature_matrix(o, h, lo, cl, vo, windows=cfg.feature_windows)
    x_obs = rolling_zscore_causal(x_obs, window=min(256, L))
    lr = np.diff(np.log(np.maximum(c, 1e-12)))
    r_cur = soft_regime_from_returns(lr, num_regimes=cfg.num_regime_dims)
    anchor = now or datetime.now(UTC)
    x_known = known_future_features(anchor, cfg.forecast_horizon, base_interval_seconds=cfg.base_interval_seconds)
    y_hat, diag = forward_numpy_reference(x_obs, x_known, r_cur, cfg, seed=seed)
    H = cfg.forecast_horizon
    q_lo = [float(y_hat[h, 0]) for h in range(H)]
    q_md = [float(y_hat[h, 1]) for h in range(H)]
    q_hi = [float(y_hat[h, 2]) for h in range(H)]
    iv = [q_hi[i] - q_lo[i] for i in range(H)]
    vol = float(np.std(lr[-32:])) if len(lr) >= 8 else 1.0
    conf = float(np.mean(np.abs(q_md)) / (np.mean(iv) + 1e-9))
    ens = [1e-8 * (i + 1) for i in range(H)]

    return ForecastPacket(
        timestamp=anchor,
        horizons=list(range(1, H + 1)),
        q_low=q_lo,
        q_med=q_md,
        q_high=q_hi,
        interval_width=iv,
        regime_vector=r_cur.tolist(),
        confidence_score=conf,
        ensemble_variance=ens,
        ood_score=min(1.0, vol),
        forecast_diagnostics={"methodology": "numpy_reference", **diag},
    )


def _empty_packet(cfg: ForecasterConfig, now: datetime | None) -> ForecastPacket:
    H = cfg.forecast_horizon
    z = [0.0] * H
    return ForecastPacket(
        timestamp=now or datetime.now(UTC),
        horizons=list(range(1, H + 1)),
        q_low=z.copy(),
        q_med=z.copy(),
        q_high=z.copy(),
        interval_width=z.copy(),
        regime_vector=[0.25] * cfg.num_regime_dims,
        confidence_score=0.0,
        ensemble_variance=z.copy(),
        ood_score=1.0,
        forecast_diagnostics={"methodology": "empty", "reason": "insufficient_history"},
    )
