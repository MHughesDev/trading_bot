"""Multi-resolution xLSTM backbone (human forecaster spec §12)."""

from __future__ import annotations

import numpy as np

from forecaster_model.models.xlstm_cell import forward_xlstm_sequence, forward_xlstm_sequence_weights


def forward_multi_resolution_xlstm(
    z_seq: np.ndarray,
    scales: tuple[int, ...],
    hidden_dim: int,
    rng: np.random.Generator,
) -> dict[int, np.ndarray]:
    """
    Z_seq [L, D_in] -> branch outputs H_s: [L, D_hidden] per scale (spec §12.4–§12.5).

    Downsampling per branch; repeat-align to base length L (spec §12.5 default).
    """
    L, _D = z_seq.shape
    branches: dict[int, np.ndarray] = {}
    for s in scales:
        if s <= 1:
            ds = z_seq
        else:
            idx = np.arange(0, L, s, dtype=int)
            ds = z_seq[idx] if len(idx) > 0 else z_seq[:1]
        h_s = forward_xlstm_sequence(ds, hidden_dim, rng)
        Ls = h_s.shape[0]
        aligned = np.zeros((L, hidden_dim))
        for t in range(L):
            src = min(int(t / max(s, 1)), Ls - 1)
            aligned[t] = h_s[src]
        branches[s] = aligned
    return branches


def forward_multi_resolution_xlstm_weights(
    z_seq: np.ndarray,
    scales: tuple[int, ...],
    hidden_dim: int,
    xlstm_by_scale: dict[int, dict[str, np.ndarray]],
) -> dict[int, np.ndarray]:
    """Multi-branch xLSTM with per-scale fixed weights (FB-SPEC-02)."""
    L, _D = z_seq.shape
    branches: dict[int, np.ndarray] = {}
    for s in scales:
        if s <= 1:
            ds = z_seq
        else:
            idx = np.arange(0, L, s, dtype=int)
            ds = z_seq[idx] if len(idx) > 0 else z_seq[:1]
        w = xlstm_by_scale[s]
        h_s = forward_xlstm_sequence_weights(
            ds,
            hidden_dim,
            w["W_i"],
            w["W_f"],
            w["W_o"],
            w["W_c"],
        )
        Ls = h_s.shape[0]
        aligned = np.zeros((L, hidden_dim))
        for t in range(L):
            src = min(int(t / max(s, 1)), Ls - 1)
            aligned[t] = h_s[src]
        branches[s] = aligned
    return branches
