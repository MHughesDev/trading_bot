"""
Temporal Fusion Transformer forecast — V1 uses a calibrated linear surrogate on lag features.

Full PyTorch TFT can replace `predict` when `nautilus-monster[models_torch]` is installed.
"""

from __future__ import annotations

import logging
from typing import Any

import numpy as np
from sklearn.linear_model import Ridge

from app.contracts.forecast import ForecastOutput

logger = logging.getLogger(__name__)


class TemporalFusionForecaster:
    """Predicts multi-horizon returns + volatility + uncertainty from a feature vector."""

    def __init__(self, feature_dim: int = 32, seed: int = 42) -> None:
        self._feature_dim = feature_dim
        self._models: dict[str, Ridge] = {
            "r1": Ridge(alpha=1.0),
            "r3": Ridge(alpha=1.0),
            "r5": Ridge(alpha=1.0),
            "r15": Ridge(alpha=1.0),
        }
        self._fitted = False
        self._X_buf: list[np.ndarray] = []
        self._y_buf: list[np.ndarray] = []

    def partial_fit_buffer(
        self,
        X: np.ndarray,
        y_future_returns: np.ndarray,
    ) -> None:
        """Accumulate supervised samples: y columns [ret+1, ret+3, ret+5, ret+15] aligned to bars."""
        self._X_buf.append(X)
        self._y_buf.append(y_future_returns)

    def fit(self) -> None:
        if not self._X_buf:
            logger.warning("TFT surrogate: no training data; using zero forecast")
            self._fitted = False
            return
        X = np.vstack(self._X_buf)
        Y = np.vstack(self._y_buf)
        if X.shape[0] < 10:
            self._fitted = False
            return
        self._models["r1"].fit(X, Y[:, 0])
        self._models["r3"].fit(X, Y[:, 1])
        self._models["r5"].fit(X, Y[:, 2])
        self._models["r15"].fit(X, Y[:, 3])
        self._fitted = True

    def predict(self, features: np.ndarray) -> ForecastOutput:
        x = features.reshape(1, -1) if features.ndim == 1 else features[-1:]
        if not self._fitted:
            return ForecastOutput(
                returns_1=0.0,
                returns_3=0.0,
                returns_5=0.0,
                returns_15=0.0,
                volatility=float(np.std(x[0]) if x.size else 0.0),
                uncertainty=1.0,
            )
        r1 = float(self._models["r1"].predict(x)[0])
        r3 = float(self._models["r3"].predict(x)[0])
        r5 = float(self._models["r5"].predict(x)[0])
        r15 = float(self._models["r15"].predict(x)[0])
        vol = float(np.std(x[0]))
        unc = float(1.0 / (1.0 + vol))
        return ForecastOutput(
            returns_1=r1,
            returns_3=r3,
            returns_5=r5,
            returns_15=r15,
            volatility=vol,
            uncertainty=unc,
        )

    def serialize(self) -> dict[str, Any]:
        return {"fitted": self._fitted, "feature_dim": self._feature_dim}
