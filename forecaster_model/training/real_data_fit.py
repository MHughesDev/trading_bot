"""
Train forecaster surrogates on **real** OHLCV only: walk-forward windows, causal features,
sklearn QuantileRegressor per (horizon, quantile). Persists joblib for inference.

Aligns with initial campaign spec (walk-forward, quantile loss family) without synthetic data.
"""

from __future__ import annotations

import json
import logging
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import numpy as np
import polars as pl
from sklearn.linear_model import QuantileRegressor

from app.contracts.forecast_packet import ForecastPacket
from forecaster_model.config import ForecasterConfig
from forecaster_model.features.normalization import rolling_zscore_causal
from forecaster_model.features.ohlc import build_observed_feature_matrix
from forecaster_model.regime.soft import soft_regime_from_returns

logger = logging.getLogger(__name__)


@dataclass
class QuantileForecasterArtifact:
    """Persisted real-data fit (not the legacy Ridge TFT)."""

    feature_dim: int
    horizons: list[int]
    quantiles: tuple[float, ...]
    models: dict[str, Any]  # key f"h{h}_q{q}" -> QuantileRegressor
    config_snapshot: dict[str, Any]
    data_snapshot_id: str | None = None

    def save(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        import joblib

        joblib.dump(
            {
                "feature_dim": self.feature_dim,
                "horizons": self.horizons,
                "quantiles": self.quantiles,
                "models": self.models,
                "config_snapshot": self.config_snapshot,
                "data_snapshot_id": self.data_snapshot_id,
                "kind": "quantile_ohlc_v1",
            },
            path,
        )

    @classmethod
    def load(cls, path: Path) -> QuantileForecasterArtifact:
        import joblib

        d = joblib.load(path)
        return cls(
            feature_dim=int(d["feature_dim"]),
            horizons=list(d["horizons"]),
            quantiles=tuple(float(x) for x in d["quantiles"]),
            models=d["models"],
            config_snapshot=dict(d.get("config_snapshot", {})),
            data_snapshot_id=d.get("data_snapshot_id"),
        )


def _feature_row_at_t(
    o: np.ndarray,
    h: np.ndarray,
    lo: np.ndarray,
    cl: np.ndarray,
    vo: np.ndarray,
    t: int,
    cfg: ForecasterConfig,
) -> np.ndarray:
    """Causal features at end index t (inclusive): last row of normalized x_obs + soft regime."""
    sl = slice(0, t + 1)
    x_obs = build_observed_feature_matrix(
        o[sl], h[sl], lo[sl], cl[sl], vo[sl], windows=cfg.feature_windows
    )
    x_obs = rolling_zscore_causal(x_obs, window=min(256, x_obs.shape[0]))
    lr = np.diff(np.log(np.maximum(cl[sl], 1e-12)))
    r_cur = soft_regime_from_returns(lr, num_regimes=cfg.num_regime_dims)
    last = x_obs[-1].astype(np.float64)
    return np.concatenate([last, r_cur])


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


def predict_quantile_forecast_packet(
    o: np.ndarray,
    h: np.ndarray,
    lo: np.ndarray,
    cl: np.ndarray,
    vo: np.ndarray,
    artifact: QuantileForecasterArtifact,
    cfg: ForecasterConfig,
    *,
    now_ts: Any,
) -> ForecastPacket:
    """Build `ForecastPacket` from trained quantile models at the last bar."""
    t = len(cl) - 1
    if t < cfg.history_length - 1:
        raise ValueError("insufficient history for prediction")
    x = _feature_row_at_t(o, h, lo, cl, vo, t, cfg)
    if x.shape[0] != artifact.feature_dim:
        raise ValueError("feature_dim mismatch artifact vs live features")
    q_lo: list[float] = []
    q_md: list[float] = []
    q_hi: list[float] = []
    for hstep in artifact.horizons:
        q_lo.append(float(artifact.models[f"h{hstep}_q{artifact.quantiles[0]}"].predict(x.reshape(1, -1))[0]))
        q_md.append(float(artifact.models[f"h{hstep}_q{artifact.quantiles[1]}"].predict(x.reshape(1, -1))[0]))
        q_hi.append(float(artifact.models[f"h{hstep}_q{artifact.quantiles[2]}"].predict(x.reshape(1, -1))[0]))
    for i in range(len(q_lo)):
        a, b, c = sorted([q_lo[i], q_md[i], q_hi[i]])
        q_lo[i], q_md[i], q_hi[i] = a, b, c
    iv = [q_hi[i] - q_lo[i] for i in range(len(q_lo))]
    lr = np.diff(np.log(np.maximum(cl.ravel(), 1e-12)))
    rv = soft_regime_from_returns(lr, num_regimes=cfg.num_regime_dims).tolist()
    conf = float(np.mean(np.abs(np.array(q_md))) / (np.mean(iv) + 1e-9))
    return ForecastPacket(
        timestamp=now_ts,
        horizons=list(artifact.horizons),
        q_low=q_lo,
        q_med=q_md,
        q_high=q_hi,
        interval_width=iv,
        regime_vector=rv,
        confidence_score=min(1.0, conf),
        ensemble_variance=[1e-8] * len(q_md),
        ood_score=float(np.std(lr[-32:])) if len(lr) >= 8 else 1.0,
        forecast_diagnostics={
            "kind": "quantile_ohlc_v1",
            "data_snapshot_id": artifact.data_snapshot_id,
            "config": artifact.config_snapshot,
        },
        packet_schema_version=1,
        source_checkpoint_id=None,
    )


def save_training_report(path: Path, report: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, default=str), encoding="utf-8")
