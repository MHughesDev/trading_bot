"""Critic / value head — human policy spec §10.3. Trainable bounded value head V(s) ∈ (-1, 1)."""

from __future__ import annotations

import numpy as np

from legacy.decision_pipeline.policy_model.objects import PolicyObservation


class ValueCritic:
    """Scalar V(s) = tanh(x · w) over flattened forecast features; trained by SGD regression."""

    def __init__(self, seed: int = 0) -> None:
        self._rng = np.random.default_rng(seed)
        self._w = self._rng.normal(0, 0.05, size=128)

    def _features(self, obs: PolicyObservation) -> np.ndarray:
        ff = np.asarray(obs.forecast_features, dtype=np.float64)
        return np.pad(ff, (0, max(0, 128 - len(ff))))[:128]

    def forward(self, obs: PolicyObservation) -> float:
        return float(np.tanh(self._features(obs) @ self._w))

    def update(self, obs: PolicyObservation, target: float, *, lr: float = 0.05) -> float:
        """One normalized-LMS step minimizing (V(s) - target)^2; returns the squared error.

        Normalizing by ``‖x‖²`` makes the step scale-invariant so a large-magnitude feature
        vector cannot overshoot and saturate ``tanh`` (which would freeze the gradient).
        """
        x = self._features(obs)
        v = float(np.tanh(x @ self._w))
        err = v - float(target)
        d_s = 2.0 * err * (1.0 - v * v)  # dL/d(pre-activation)
        self._w -= lr * d_s * x / (float(x @ x) + 1e-8)
        return float(err * err)
