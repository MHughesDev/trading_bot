"""Inference-only quantile forecaster path (Phase E / P3).

Separates the *serve* side (load + predict) from the *train* side
(``training_pipeline.forecaster_training.real_data_fit``).  Serving should import from this module;
training imports from ``real_data_fit`` which defines the fitting logic.

**sklearn on the serving path:** joblib deserialisation requires sklearn to be installed
(Python needs to instantiate ``QuantileRegressor`` objects when unpickling).  That dependency
is unavoidable.  What P3 removes is the module-level ``from sklearn.linear_model import
QuantileRegressor`` in the serving import chain — the import now happens lazily inside
``QuantileForecasterArtifact.load()`` (via joblib) rather than at module load time.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

import numpy as np

from app.contracts.forecast_packet import ForecastPacket
from legacy.decision_pipeline.forecaster_model.config import ForecasterConfig
from legacy.decision_pipeline.forecaster_model.features.normalization import rolling_zscore_causal
from legacy.decision_pipeline.forecaster_model.features.ohlc import build_observed_feature_matrix
from legacy.decision_pipeline.forecaster_model.regime.soft import soft_regime_from_returns


@dataclass
class QuantileForecasterArtifact:
    """Persisted real-data fit — inference side (load + predict).

    The training constructor lives in ``training_pipeline.forecaster_training.real_data_fit``
    so that ``sklearn`` is only imported on the training path.
    """

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
    """Causal features at end index t (inclusive): last row of normalised x_obs + soft regime."""
    sl = slice(0, t + 1)
    x_obs = build_observed_feature_matrix(
        o[sl], h[sl], lo[sl], cl[sl], vo[sl], windows=cfg.feature_windows
    )
    x_obs = rolling_zscore_causal(x_obs, window=min(256, x_obs.shape[0]))
    lr = np.diff(np.log(np.maximum(cl[sl], 1e-12)))
    r_cur = soft_regime_from_returns(lr, num_regimes=cfg.num_regime_dims)
    last = x_obs[-1].astype(np.float64)
    return np.concatenate([last, r_cur])


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
    """Build ``ForecastPacket`` from trained quantile models at the last bar."""
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
