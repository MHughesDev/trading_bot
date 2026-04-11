"""Regime-conditioned fusion (human forecaster spec §13)."""

from __future__ import annotations

import numpy as np


def _softmax(x: np.ndarray, axis: int = -1) -> np.ndarray:
    e = np.exp(x - np.max(x, axis=axis, keepdims=True))
    return e / np.sum(e, axis=axis, keepdims=True)


def forward_regime_conditioned_fusion(
    branch_outputs: dict[int, np.ndarray],
    r_cur: np.ndarray,
    rng: np.random.Generator,
) -> tuple[np.ndarray, np.ndarray]:
    """
    H_stack per spec §13.2; alpha = softmax(MLP(R_cur)); fused = sum_s alpha_s H_s.
    """
    scales = sorted(branch_outputs.keys())
    L, Hdim = branch_outputs[scales[0]].shape
    S = len(scales)
    W = rng.normal(0, 0.1, size=(len(r_cur), S))
    return forward_regime_conditioned_fusion_weights(branch_outputs, r_cur, W)


def forward_regime_conditioned_fusion_weights(
    branch_outputs: dict[int, np.ndarray],
    r_cur: np.ndarray,
    W: np.ndarray,
) -> tuple[np.ndarray, np.ndarray]:
    """Fusion with fixed `W` [R, S] (FB-SPEC-02)."""
    scales = sorted(branch_outputs.keys())
    L, Hdim = branch_outputs[scales[0]].shape
    S = len(scales)
    if W.shape != (len(r_cur), S):
        raise ValueError(f"fusion W shape {W.shape} expected ({len(r_cur)}, {S})")
    alpha = _softmax(r_cur @ W)
    fused = np.zeros((L, Hdim))
    for i, s in enumerate(scales):
        fused += alpha[i] * branch_outputs[s]
    return fused, alpha
