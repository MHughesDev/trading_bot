"""Probabilistic target construction (I-1.4).

Provides triple-barrier, quantile, and devolatized labelers used by
distributional adapters. All labelers operate on a price/return series
and emit a label array aligned with the input index.

Also emits `label_overlap_bars` — the effective forward-label horizon in
bars — so the walk-forward purge geometry (Phase 0) remains correct.
"""

from __future__ import annotations

import numpy as np
import pandas as pd


# --------------------------------------------------------------------------- #
# Utility
# --------------------------------------------------------------------------- #

def _returns_from_df(df: pd.DataFrame, close_col: str = "close") -> np.ndarray:
    """Simple log-return series from a close price column."""
    close = df[close_col].to_numpy(dtype=float) if close_col in df.columns else None
    if close is None or len(close) < 2:
        # Fall back to existing label column if close is unavailable.
        if "label" in df.columns:
            return df["label"].to_numpy(dtype=float)
        return np.zeros(len(df))
    log_returns = np.concatenate([[0.0], np.diff(np.log(close + 1e-12))])
    return log_returns


def realized_vol(returns: np.ndarray, window: int = 20) -> np.ndarray:
    """Rolling realized volatility (std of returns) over `window` bars.

    Uses an expanding window for the first `window` bars.
    """
    n = len(returns)
    out = np.full(n, 1.0)
    for i in range(n):
        start = max(0, i - window + 1)
        chunk = returns[start : i + 1]
        out[i] = float(np.std(chunk)) if len(chunk) > 1 else 1.0
    out = np.where(out < 1e-8, 1e-8, out)
    return out


# --------------------------------------------------------------------------- #
# Triple-barrier labeling (I-1.4)
# --------------------------------------------------------------------------- #

def triple_barrier_labels(
    df: pd.DataFrame,
    pt: float = 2.0,
    sl: float = 1.0,
    vert_bars: int = 60,
    devol: bool = True,
    close_col: str = "close",
) -> tuple[np.ndarray, int]:
    """Compute triple-barrier labels.

    Returns:
        (labels, label_overlap_bars) where:
          - labels[i] =  1 (profit target hit first)
          -              0 (vertical barrier — time exit)
          -             -1 (stop loss hit first)
        label_overlap_bars = vert_bars (the effective forward-label horizon)

    Parameters:
        pt:         profit-target multiplier (in units of realized vol × price)
        sl:         stop-loss multiplier (same units)
        vert_bars:  vertical barrier in bars
        devol:      scale barriers by realized vol when True
    """
    log_ret = _returns_from_df(df, close_col)
    vol = realized_vol(log_ret) if devol else np.ones(len(log_ret))
    n = len(df)
    labels = np.zeros(n, dtype=np.float32)

    for i in range(n):
        if i + vert_bars >= n:
            labels[i] = 0.0
            continue
        horizon_rets = log_ret[i + 1 : i + vert_bars + 1]
        cum = np.cumsum(horizon_rets)
        barrier_up = pt * vol[i]
        barrier_dn = -sl * vol[i]
        for j, c in enumerate(cum):
            if c >= barrier_up:
                labels[i] = 1.0
                break
            if c <= barrier_dn:
                labels[i] = -1.0
                break
        # else: vertical exit → 0

    return labels, vert_bars


# --------------------------------------------------------------------------- #
# Quantile target (I-1.4)
# --------------------------------------------------------------------------- #

def quantile_labels(
    df: pd.DataFrame,
    horizon_bars: int = 60,
    quantile: float = 0.5,
    close_col: str = "close",
) -> tuple[np.ndarray, int]:
    """Forward return at a given quantile over rolling `horizon_bars`.

    The label at bar i is the `quantile`-quantile of returns over
    [i+1, i+horizon_bars], estimating the realized distribution.
    In practice this reduces to the single forward return when horizon=1.
    Returns (labels, label_overlap_bars).
    """
    log_ret = _returns_from_df(df, close_col)
    n = len(log_ret)
    labels = np.zeros(n, dtype=np.float32)
    for i in range(n - horizon_bars):
        window = log_ret[i + 1 : i + horizon_bars + 1]
        labels[i] = float(np.quantile(window, quantile))
    return labels, horizon_bars


# --------------------------------------------------------------------------- #
# Devolatized forward-return label (I-1.4)
# --------------------------------------------------------------------------- #

def devolatized_labels(
    df: pd.DataFrame,
    horizon_bars: int = 60,
    vol_window: int = 20,
    close_col: str = "close",
) -> tuple[np.ndarray, int, float]:
    """Forward log-return divided by realized vol on the training window.

    Returns (labels, label_overlap_bars, sigma_train).
    `sigma_train` is the mean realized vol over the series — a stable
    scale estimate to store in the bundle for serve-time rescaling.
    """
    log_ret = _returns_from_df(df, close_col)
    vol = realized_vol(log_ret, window=vol_window)
    n = len(log_ret)
    labels = np.zeros(n, dtype=np.float32)
    for i in range(n - horizon_bars):
        fwd_ret = float(np.sum(log_ret[i + 1 : i + horizon_bars + 1]))
        labels[i] = fwd_ret / vol[i]
    sigma_train = float(np.mean(vol[: n // 2]))  # first half = train estimate
    return labels, horizon_bars, sigma_train
