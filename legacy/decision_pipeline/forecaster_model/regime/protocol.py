"""Regime estimator interface (human spec §9.4)."""

from __future__ import annotations

from typing import Protocol, runtime_checkable

import numpy as np


@runtime_checkable
class RegimeEstimatorProtocol(Protocol):
    def fit(self, features: np.ndarray) -> None: ...

    def update(self, feature_row: np.ndarray) -> None: ...

    def predict_proba(self, feature_row: np.ndarray) -> np.ndarray:
        """Return soft regime vector [F_regime]."""
        ...
