"""xLSTM branch cell (human forecaster spec §12.4–§12.7).

Spec §12.7: If a strict xLSTM implementation is unavailable, the abstraction permits
drop-in replacement with an enhanced LSTM cell exposing the same interface.

This module implements a **standard LSTM** forward pass in NumPy as the **recurrent core**
for each branch until a full xLSTM cell is integrated. Call sites use `forward_xlstm_sequence`
so the multi-resolution stack matches the spec's `xLSTM_s` contract by name.

**FB-FR-PG7 (ablation):** `ABLATION_SIGNED_OFF_LSTM_BASELINE` documents the shipped baseline
until a native xLSTM or third-party cell is integrated and evaluated per spec §27.
"""

from __future__ import annotations

import numpy as np

# Formal LSTM baseline sign-off flag (spec §27 ablation matrix item 7).
ABLATION_SIGNED_OFF_LSTM_BASELINE = True


def forward_xlstm_sequence(x: np.ndarray, hidden_dim: int, rng: np.random.Generator) -> np.ndarray:
    """
    Process sequence X: [L, D_in] -> H: [L, D_hidden].

    Maps to spec: H_s = xLSTM_s(X_s) (behavioral recurrent state evolution).
    """
    L, Fin = x.shape
    W_i = rng.normal(0, 0.05, size=(Fin + hidden_dim, hidden_dim))
    W_f = rng.normal(0, 0.05, size=(Fin + hidden_dim, hidden_dim))
    W_o = rng.normal(0, 0.05, size=(Fin + hidden_dim, hidden_dim))
    W_c = rng.normal(0, 0.05, size=(Fin + hidden_dim, hidden_dim))
    return forward_xlstm_sequence_weights(x, hidden_dim, W_i, W_f, W_o, W_c)


def forward_xlstm_sequence_weights(
    x: np.ndarray,
    hidden_dim: int,
    W_i: np.ndarray,
    W_f: np.ndarray,
    W_o: np.ndarray,
    W_c: np.ndarray,
) -> np.ndarray:
    """LSTM forward with fixed gate weights (FB-SPEC-02)."""
    L, Fin = x.shape
    exp = (Fin + hidden_dim, hidden_dim)
    for name, W in (("W_i", W_i), ("W_f", W_f), ("W_o", W_o), ("W_c", W_c)):
        if W.shape != exp:
            raise ValueError(f"{name} shape {W.shape} expected {exp}")
    h = np.zeros(hidden_dim)
    c = np.zeros(hidden_dim)
    out = np.zeros((L, hidden_dim))
    for t in range(L):
        xh = np.concatenate([x[t], h])
        i_gate = 1 / (1 + np.exp(-(xh @ W_i)))
        f_gate = 1 / (1 + np.exp(-(xh @ W_f)))
        o_gate = 1 / (1 + np.exp(-(xh @ W_o)))
        c_tilde = np.tanh(xh @ W_c)
        c = f_gate * c + i_gate * c_tilde
        h = o_gate * np.tanh(c)
        out[t] = h
    return out
