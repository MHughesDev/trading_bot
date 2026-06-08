"""Sliding windows and log-return targets (human spec §4, §18)."""

from __future__ import annotations

import numpy as np


def future_log_returns(close: np.ndarray, horizons: list[int], *, eps: float = 1e-12) -> np.ndarray:
    """
    At index t (last history index), y[h] = log(P_{t+h} / P_t) for each horizon step count h.
    close: 1D array, horizons are **steps ahead** (1-based steps from t).
    """
    c = np.asarray(close, dtype=np.float64).ravel()
    t = len(c) - 1
    out = np.zeros(len(horizons), dtype=np.float64)
    for i, h in enumerate(horizons):
        j = t + h
        if j >= len(c):
            out[i] = np.nan
        else:
            out[i] = float(np.log(max(c[j], eps) / max(c[t], eps)))
    return out


def sliding_window_indices(n: int, history_len: int, step: int = 1) -> list[tuple[int, int]]:
    """Valid (start, end_exclusive) for history slices [start:end] with end <= n."""
    out: list[tuple[int, int]] = []
    for end in range(history_len, n + 1, step):
        out.append((end - history_len, end))
    return out
