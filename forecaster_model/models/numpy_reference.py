"""
Reference forecaster forward path (NumPy): VSN → causal CNN → multi-branch RNN → regime fusion → quantile head.

Recurrent core: **LSTM-style** forward as behavioral stand-in for xLSTM (human spec §12.7).
"""

from __future__ import annotations

import numpy as np

from forecaster_model.config import ForecasterConfig


def _softmax(x: np.ndarray, axis: int = -1) -> np.ndarray:
    e = np.exp(x - np.max(x, axis=axis, keepdims=True))
    return e / np.sum(e, axis=axis, keepdims=True)


def variable_selection_gates(x: np.ndarray, rng: np.random.Generator) -> tuple[np.ndarray, np.ndarray]:
    """VSN (spec §10): g_t = softmax(W x_t) over features, x_vsn = g ⊙ x. x: [L, F]."""
    L, F = x.shape
    W = rng.normal(0, 0.05, size=(F, F))
    gates = np.zeros((L, F))
    for t in range(L):
        gates[t] = _softmax(W @ x[t])
    return x * gates, gates


def causal_conv_stack(x: np.ndarray, rng: np.random.Generator, latent_dim: int = 32) -> np.ndarray:
    """
    Causal latent encoder (spec §11): three length-preserving convs via valid conv + left padding.
    x: [L, F] -> z: [L, D_latent]
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


def _vanilla_rnn(x: np.ndarray, hidden_dim: int, rng: np.random.Generator) -> np.ndarray:
    """Simple RNN: h_t = tanh(W x_t + U h_{t-1}). x: [L, Fin] -> [L, H]."""
    L, Fin = x.shape
    W = rng.normal(0, 0.05, size=(Fin, hidden_dim))
    U = rng.normal(0, 0.05, size=(hidden_dim, hidden_dim))
    h = np.zeros(hidden_dim)
    out = np.zeros((L, hidden_dim))
    for t in range(L):
        h = np.tanh(x[t] @ W + h @ U)
        out[t] = h
    return out


def multi_resolution_branches(
    z_seq: np.ndarray,
    scales: tuple[int, ...],
    hidden_dim: int,
    rng: np.random.Generator,
) -> dict[int, np.ndarray]:
    """Downsample → RNN → repeat-align to L (spec §12.5)."""
    L, D = z_seq.shape
    branches: dict[int, np.ndarray] = {}
    for s in scales:
        if s <= 1:
            ds = z_seq
        else:
            idx = np.arange(0, L, s, dtype=int)
            ds = z_seq[idx] if len(idx) > 0 else z_seq[:1]
        h_s = _vanilla_rnn(ds, hidden_dim, rng)
        Ls = h_s.shape[0]
        aligned = np.zeros((L, hidden_dim))
        for t in range(L):
            src = min(int(t / max(s, 1)), Ls - 1)
            aligned[t] = h_s[src]
        branches[s] = aligned
    return branches


def regime_conditioned_fusion(
    branch_outputs: dict[int, np.ndarray],
    r_cur: np.ndarray,
    rng: np.random.Generator,
) -> tuple[np.ndarray, np.ndarray]:
    """spec §13: alpha = softmax(W r), H_fused = sum_s alpha_s H_s."""
    scales = sorted(branch_outputs.keys())
    L, Hdim = branch_outputs[scales[0]].shape
    S = len(scales)
    W = rng.normal(0, 0.1, size=(len(r_cur), S))
    alpha = _softmax(r_cur @ W)
    fused = np.zeros((L, Hdim))
    for i, s in enumerate(scales):
        fused += alpha[i] * branch_outputs[s]
    return fused, alpha


def quantile_decoder(
    h_fused_last: np.ndarray,
    x_known: np.ndarray,
    quantiles: tuple[float, ...],
    rng: np.random.Generator,
) -> np.ndarray:
    """spec §14: per-horizon quantiles from [h_summary || x_known[h]]."""
    H, Fk = x_known.shape
    Qn = len(quantiles)
    D = len(h_fused_last)
    out = np.zeros((H, Qn))
    W = rng.normal(0, 0.05, size=(D + Fk, Qn))
    for h in range(H):
        inp = np.concatenate([h_fused_last, x_known[h]])
        q = inp @ W
        # order: low, med, high
        med = q[1]
        out[h, 0] = med - np.exp(q[0])
        out[h, 1] = med
        out[h, 2] = med + np.exp(q[2])
    for h in range(H):
        a, b, c = sorted(out[h])
        out[h, 0], out[h, 1], out[h, 2] = a, b, c
    return out


def forward_numpy_reference(
    x_obs: np.ndarray,
    x_known: np.ndarray,
    r_cur: np.ndarray,
    cfg: ForecasterConfig | None = None,
    *,
    seed: int = 42,
) -> tuple[np.ndarray, dict]:
    """
    x_obs [L, F_obs], x_known [H, F_known], r_cur [F_regime] -> y_hat_q [H, Qn].
    """
    cfg = cfg or ForecasterConfig()
    rng = np.random.default_rng(seed)
    x_vsn, gates = variable_selection_gates(x_obs, rng)
    z_seq = causal_conv_stack(x_vsn, rng)
    L = z_seq.shape[0]
    scales = (1, 4, 16) if L >= 16 else (1,)
    branches = multi_resolution_branches(z_seq, scales, hidden_dim=32, rng=rng)
    fused, alpha = regime_conditioned_fusion(branches, r_cur, rng)
    h_last = fused[-1]
    y_hat = quantile_decoder(h_last, x_known, cfg.quantiles, rng)
    return y_hat, {
        "vsn_gate_mean": float(gates.mean()),
        "fusion_alpha": alpha.tolist(),
        "branch_scales": list(scales),
    }
