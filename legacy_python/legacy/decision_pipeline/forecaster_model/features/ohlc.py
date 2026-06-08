"""Observed-feature construction from OHLCV (human spec §7) — numpy, no lookahead."""

from __future__ import annotations

import numpy as np


def log_returns(close: np.ndarray, eps: float = 1e-12) -> np.ndarray:
    """Per-step log return; first row NaN → 0."""
    c = np.asarray(close, dtype=np.float64).ravel()
    lr = np.diff(np.log(np.maximum(c, eps)))
    return np.concatenate([[0.0], lr])


def rolling_mean(x: np.ndarray, k: int) -> np.ndarray:
    x = np.asarray(x, dtype=np.float64).ravel()
    if k <= 1:
        return x.copy()
    out = np.zeros_like(x)
    for i in range(len(x)):
        a = max(0, i - k + 1)
        out[i] = float(np.mean(x[a : i + 1]))
    return out


def rolling_std(x: np.ndarray, k: int) -> np.ndarray:
    x = np.asarray(x, dtype=np.float64).ravel()
    if k <= 1:
        return np.zeros_like(x)
    out = np.zeros_like(x)
    for i in range(len(x)):
        a = max(0, i - k + 1)
        out[i] = float(np.std(x[a : i + 1])) if i - a + 1 > 1 else 0.0
    return out


def build_observed_feature_matrix(
    open_: np.ndarray,
    high: np.ndarray,
    low: np.ndarray,
    close: np.ndarray,
    volume: np.ndarray,
    *,
    windows: tuple[int, ...] = (4, 16, 64),
    eps: float = 1e-12,
) -> np.ndarray:
    """
    Build [L, F_obs] matrix for the last L rows (caller slices OHLCV to history window).

    Feature groups: base returns, abs return, range ratio, volume change, rolling stats per window.
    """
    o = np.asarray(open_, dtype=np.float64).ravel()
    h = np.asarray(high, dtype=np.float64).ravel()
    lo = np.asarray(low, dtype=np.float64).ravel()
    c = np.asarray(close, dtype=np.float64).ravel()
    v = np.asarray(volume, dtype=np.float64).ravel()
    n = len(c)
    if not (len(o) == len(h) == len(lo) == len(c) == len(v) == n):
        raise ValueError("OHLCV length mismatch")

    lr1 = log_returns(c, eps=eps)
    abs_r = np.abs(lr1)
    co = np.log(np.maximum(c, eps) / np.maximum(o, eps))
    rng = (h - lo) / np.maximum(np.abs(c), eps)
    vch = np.log(np.maximum(v[1:], eps) / np.maximum(v[:-1], eps))
    vch = np.concatenate([[0.0], vch])

    cols: list[np.ndarray] = [lr1, abs_r, co, rng, vch]
    for w in windows:
        cols.append(rolling_mean(lr1, w))
        cols.append(rolling_std(lr1, w))
    mat = np.stack(cols, axis=1)
    mat = np.nan_to_num(mat, nan=0.0, posinf=0.0, neginf=0.0)
    return mat
