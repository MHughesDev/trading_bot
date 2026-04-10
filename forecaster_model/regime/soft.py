"""Soft regime vector (human spec §9) — rule-based first implementation."""

from __future__ import annotations

import numpy as np


def soft_regime_from_returns(
    log_returns: np.ndarray,
    *,
    num_regimes: int = 4,
    vol_window: int = 32,
) -> np.ndarray:
    """
    Map recent return stream to a soft 4-vector:
    [uptrend, downtrend, low-vol chop, high-vol chop] (human spec §9.2).
    """
    lr = np.asarray(log_returns, dtype=np.float64).ravel()
    if len(lr) < 2:
        return np.ones(num_regimes, dtype=np.float64) / num_regimes

    mu = float(np.mean(lr[-vol_window:]))
    sig = float(np.std(lr[-vol_window:])) + 1e-12
    mom = float(np.mean(lr[-8:])) if len(lr) >= 8 else mu

    z_trend = mom / sig
    z_vol = sig / (float(np.median(np.abs(lr[-vol_window:]))) + 1e-12)

    # Soft scores (positive = uptrend / high vol)
    p_up = float(1.0 / (1.0 + np.exp(-z_trend)))
    p_down = 1.0 - p_up
    p_high_chop = float(1.0 / (1.0 + np.exp(-(z_vol - 1.5))))
    p_low_chop = 1.0 - p_high_chop

    raw = np.array([p_up * p_low_chop, p_down * p_low_chop, p_low_chop * 0.5, p_high_chop], dtype=np.float64)
    if num_regimes != 4:
        raw = np.resize(raw, num_regimes)
    s = raw.sum()
    return raw / s if s > 0 else np.ones(num_regimes) / num_regimes
