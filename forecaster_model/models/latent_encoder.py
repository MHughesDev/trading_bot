"""Causal latent encoder (human forecaster spec §11 — default causal 1D CNN)."""

from __future__ import annotations

import numpy as np


def forward_latent_encoder(x: np.ndarray, rng: np.random.Generator, latent_dim: int = 32) -> np.ndarray:
    """
    X: [L, F_in] -> Z_seq: [L, D_latent].

    Topology: Conv1D k=3,5,7 with channels per spec §11.3 (length-preserving, causal padding).
    """
    L, Fin = x.shape
    channels = (32, 64, latent_dim)
    ks = (3, 5, 7)
    h = x
    for ch_out, k in zip(channels, ks, strict=True):
        pad = np.zeros((k - 1, h.shape[1]))
        padded = np.vstack([pad, h])
        out = np.zeros((L, ch_out))
        for t in range(L):
            win = padded[t : t + k, :].ravel()
            W = rng.normal(0, 0.02, size=(win.size, ch_out))
            out[t] = np.tanh(win @ W)
        h = out
    return h
