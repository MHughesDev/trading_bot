"""Critic / value head — human policy spec §10.3 (placeholder until RL training stack)."""

from __future__ import annotations

import numpy as np

from policy_model.objects import PolicyObservation


class ValueCritic:
    """Scalar V(s) from flattened observation features (stub for actor-critic wiring)."""

    def __init__(self, seed: int = 0) -> None:
        self._rng = np.random.default_rng(seed)
        self._w = self._rng.normal(0, 0.05, size=128)

    def forward(self, obs: PolicyObservation) -> float:
        ff = np.asarray(obs.forecast_features, dtype=np.float64)
        x = np.pad(ff, (0, max(0, 128 - len(ff))))[:128]
        return float(np.tanh(x @ self._w))
