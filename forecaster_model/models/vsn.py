"""Variable Selection Network (human forecaster spec §10)."""

from __future__ import annotations

import numpy as np


def _softmax(x: np.ndarray, axis: int = -1) -> np.ndarray:
    e = np.exp(x - np.max(x, axis=axis, keepdims=True))
    return e / np.sum(e, axis=axis, keepdims=True)


def forward_vsn(x: np.ndarray, rng: np.random.Generator) -> tuple[np.ndarray, np.ndarray]:
    """
    X_obs [L, F_obs] -> X_vsn [L, F_obs], gates [L, F_obs].

    g_t = softmax(W x_t); x_vsn,t = g_t ⊙ x_t (spec §10.2).
    """
    L, F = x.shape
    W = rng.normal(0, 0.05, size=(F, F))
    return forward_vsn_weights(x, W)


def forward_vsn_weights(x: np.ndarray, W: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """Deterministic VSN when `W` is fixed (FB-SPEC-02 checkpoint path)."""
    L, F = x.shape
    if W.shape != (F, F):
        raise ValueError(f"VSN W shape {W.shape} incompatible with x F_obs={F}")
    gates = np.zeros((L, F))
    for t in range(L):
        gates[t] = _softmax(W @ x[t])
    return x * gates, gates
