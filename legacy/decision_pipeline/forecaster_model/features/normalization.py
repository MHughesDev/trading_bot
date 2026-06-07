"""Global rolling z-score normalization — uses only past data at each t (human spec §8.2, no lookahead)."""

from __future__ import annotations

import numpy as np


def rolling_zscore_causal(
    x: np.ndarray,
    window: int,
    *,
    eps: float = 1e-8,
) -> np.ndarray:
    """
    x: [L, F] or [B, L, F]. For each index t, mean/std from x[max(0,t-W+1):t+1] inclusive.

    At t=0 only current point is used (std → 0 → 0 after nan_to_num).
    """
    if x.ndim == 2:
        x = x[np.newaxis, ...]
        squeeze = True
    else:
        squeeze = False
    B, L, F = x.shape
    out = np.zeros_like(x, dtype=np.float64)
    w = max(1, int(window))
    for b in range(B):
        for t in range(L):
            a = max(0, t - w + 1)
            seg = x[b, a : t + 1, :]
            mu = seg.mean(axis=0)
            sd = seg.std(axis=0) + eps
            out[b, t, :] = (x[b, t, :] - mu) / sd
    out = np.nan_to_num(out, nan=0.0, posinf=0.0, neginf=0.0)
    return out[0] if squeeze else out


def regime_soft_blend_stats(
    x: np.ndarray,
    regime_probs: np.ndarray,
    *,
    n_regimes: int = 4,
    eps: float = 1e-8,
) -> tuple[np.ndarray, np.ndarray]:
    """
    Soft regime-conditioned normalization (human spec §8.3 soft-weight version).

    x: [L, F], regime_probs: [L, K] or [K] broadcast to time.
    Returns per-timestep (mu_tilde, sigma_tilde) each [L, F] for blending; then x'' = (x - mu_tilde) / sigma_tilde.
    """
    x = np.asarray(x, dtype=np.float64)
    L, F = x.shape
    rp = np.asarray(regime_probs, dtype=np.float64)
    if rp.ndim == 1:
        rp = np.tile(rp, (L, 1))
    K = rp.shape[1]
    # Per-regime running mean over time (causal)
    mus = np.zeros((L, F, K))
    for k in range(K):
        for t in range(L):
            mus[t, :, k] = x[: t + 1, :].mean(axis=0)
    mu_tilde = np.einsum("tk,tfk->tf", rp, mus)
    sigs = np.zeros((L, F, K))
    for k in range(K):
        for t in range(L):
            seg = x[: t + 1, :]
            sigs[t, :, k] = seg.std(axis=0) + eps
    sig_tilde = np.einsum("tk,tfk->tf", rp, sigs)
    return mu_tilde, np.maximum(sig_tilde, eps)
