"""
Train forecaster surrogates on **real** OHLCV only: walk-forward windows, causal features,
sklearn QuantileRegressor per (horizon, quantile). Persists joblib for inference.

Aligns with initial campaign spec (walk-forward, quantile loss family) without synthetic data.

**Module boundary (Phase E / P3):**
- ``QuantileForecasterArtifact`` and ``predict_quantile_forecast_packet`` live in
  ``forecaster_model.inference.quantile_infer`` — they are re-exported here for backward compat.
- Serving code should import from ``forecaster_model.inference.quantile_infer`` directly.
- This module keeps only training-specific code (sklearn fit, walk-forward splits, reports).
"""

from __future__ import annotations

import json
import logging
from pathlib import Path
from typing import Any

import numpy as np
import polars as pl
from sklearn.linear_model import QuantileRegressor

from forecaster_model.config import ForecasterConfig
from forecaster_model.inference.quantile_infer import (
    QuantileForecasterArtifact,
    _feature_row_at_t,
    predict_quantile_forecast_packet,
)

# Re-export for backward compatibility — code that imported from here keeps working.
__all__ = [
    "QuantileForecasterArtifact",
    "predict_quantile_forecast_packet",
    "fit_quantile_forecaster_from_bars",
    "save_training_report",
]

logger = logging.getLogger(__name__)


def _targets_at_t(
    close: np.ndarray,
    t: int,
    horizons: list[int],
) -> np.ndarray | None:
    c = np.asarray(close, dtype=np.float64).ravel()
    y = np.zeros(len(horizons), dtype=np.float64)
    for i, h in enumerate(horizons):
        j = t + h
        if j >= len(c):
            return None
        y[i] = float(np.log(max(c[j], 1e-12) / max(c[t], 1e-12)))
    return y


def fit_quantile_forecaster_from_bars(
    bars: pl.DataFrame,
    cfg: ForecasterConfig,
    *,
    train_range: range,
    data_snapshot_id: str | None = None,
    seed: int = 42,
) -> QuantileForecasterArtifact:
    """
    Fit QuantileRegressor models on train_range only. Bars must be sorted OHLCV with
    columns open, high, low, close, volume (timestamp optional).
    """
    _ = seed  # reserved for future torch / ensemble
    required = {"open", "high", "low", "close", "volume"}
    cols = set(bars.columns)
    if not required.issubset(cols):
        raise ValueError(f"bars must contain {required}, got {cols}")

    o = bars["open"].to_numpy()
    h = bars["high"].to_numpy()
    lo = bars["low"].to_numpy()
    cl = bars["close"].to_numpy()
    vo = bars["volume"].to_numpy()
    n = len(cl)
    H = cfg.forecast_horizon
    horizons = list(range(1, H + 1))
    L = cfg.history_length
    max_h = max(horizons)
    # First index t where we have history L and future max_h
    t_min = train_range.start + L - 1
    t_max = min(train_range.stop - 1, n - 1 - max_h)
    samples: list[tuple[np.ndarray, np.ndarray]] = []
    for t in range(t_min, t_max + 1):
        if t not in train_range:
            continue
        feat = _feature_row_at_t(o, h, lo, cl, vo, t, cfg)
        y = _targets_at_t(cl, t, horizons)
        if y is None:
            continue
        samples.append((feat, y))
    if len(samples) < 32:
        raise RuntimeError(
            f"insufficient real training samples after windowing: {len(samples)} (need >= 32)"
        )
    X = np.stack([s[0] for s in samples])
    Y = np.stack([s[1] for s in samples])
    F = X.shape[1]
    qlist = tuple(float(q) for q in cfg.quantiles)
    models: dict[str, Any] = {}
    for hi, h in enumerate(horizons):
        y_h = Y[:, hi]
        for q in qlist:
            m = QuantileRegressor(quantile=q, alpha=1e-3, solver="highs")
            m.fit(X, y_h)
            models[f"h{h}_q{q}"] = m
    snap = {
        "history_length": cfg.history_length,
        "forecast_horizon": cfg.forecast_horizon,
        "quantiles": list(cfg.quantiles),
        "feature_windows": list(cfg.feature_windows),
        "num_regime_dims": cfg.num_regime_dims,
    }
    return QuantileForecasterArtifact(
        feature_dim=F,
        horizons=horizons,
        quantiles=qlist,
        models=models,
        config_snapshot=snap,
        data_snapshot_id=data_snapshot_id,
    )


def save_training_report(path: Path, report: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, default=str), encoding="utf-8")
