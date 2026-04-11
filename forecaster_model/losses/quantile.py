"""Quantile (pinball) loss — human forecaster spec §15.1."""

from __future__ import annotations

import numpy as np


def pinball_loss_scalar(y: float, y_hat: float, q: float) -> float:
    e = y - y_hat
    return max(q * e, (q - 1.0) * e)


def quantile_loss_batch(y: np.ndarray, y_hat_q: np.ndarray, quantiles: tuple[float, ...]) -> float:
    """Mean over batch, horizons, and quantiles (spec §15.1)."""
    y = np.asarray(y, dtype=np.float64)
    pred = np.asarray(y_hat_q, dtype=np.float64)
    total = 0.0
    n = 0
    for b in range(y.shape[0]):
        for h in range(y.shape[1]):
            for j, q in enumerate(quantiles):
                total += pinball_loss_scalar(float(y[b, h]), float(pred[b, h, j]), q)
                n += 1
    return total / max(n, 1)
