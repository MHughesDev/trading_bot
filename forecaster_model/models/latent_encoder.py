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
        win_size = k * h.shape[1]
        W = rng.normal(0, 0.02, size=(win_size, ch_out))
        out = np.zeros((L, ch_out))
        for t in range(L):
            win = padded[t : t + k, :].ravel()
            out[t] = np.tanh(win @ W)
        h = out
    return h


def forward_latent_encoder_weights(
    x: np.ndarray,
    W0: np.ndarray,
    W1: np.ndarray,
    W2: np.ndarray,
    *,
    latent_dim: int = 32,
) -> np.ndarray:
    """Causal CNN stack with fixed kernels `W0`, `W1`, `W2` (FB-SPEC-02)."""
    L, Fin = x.shape
    channels = (32, 64, latent_dim)
    ks = (3, 5, 7)
    weights = (W0, W1, W2)
    h = x
    for ch_out, k, W in zip(channels, ks, weights, strict=True):
        pad = np.zeros((k - 1, h.shape[1]))
        padded = np.vstack([pad, h])
        win_size = k * h.shape[1]
        if W.shape != (win_size, ch_out):
            raise ValueError(
                f"latent layer expects W shape ({win_size}, {ch_out}), got {W.shape}"
            )
        out = np.zeros((L, ch_out))
        for t in range(L):
            win = padded[t : t + k, :].ravel()
            out[t] = np.tanh(win @ W)
        h = out
    return h
