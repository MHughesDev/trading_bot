"""Quantile decoder / forecast head (human forecaster spec §14)."""

from __future__ import annotations

import numpy as np


def forward_quantile_decoder(
    h_fused_last: np.ndarray,
    x_known: np.ndarray,
    quantiles: tuple[float, ...],
    rng: np.random.Generator,
) -> np.ndarray:
    """
    Y_hat_q: [H, Qn] from fused state summary and X_known (spec §14.2–§14.3).

    Enforces q_low <= q_med <= q_high (spec §14.5).
    """
    H, Fk = x_known.shape
    Qn = len(quantiles)
    D = len(h_fused_last)
    W = rng.normal(0, 0.05, size=(D + Fk, Qn))
    return forward_quantile_decoder_weights(h_fused_last, x_known, quantiles, W)


def forward_quantile_decoder_weights(
    h_fused_last: np.ndarray,
    x_known: np.ndarray,
    quantiles: tuple[float, ...],
    W: np.ndarray,
) -> np.ndarray:
    """Quantile head with fixed `W` (FB-SPEC-02)."""
    H, Fk = x_known.shape
    Qn = len(quantiles)
    D = len(h_fused_last)
    out = np.zeros((H, Qn))
    if W.shape != (D + Fk, Qn):
        raise ValueError(f"decoder W shape {W.shape} expected ({D + Fk}, {Qn})")
    for h in range(H):
        inp = np.concatenate([h_fused_last, x_known[h]])
        q = inp @ W
        med = q[1]
        out[h, 0] = med - np.exp(q[0])
        out[h, 1] = med
        out[h, 2] = med + np.exp(q[2])
    for h in range(H):
        a, b, c = sorted(out[h])
        out[h, 0], out[h, 1], out[h, 2] = a, b, c
    return out
