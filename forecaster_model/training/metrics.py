"""Quantile / pinball losses for forecaster training."""

from __future__ import annotations

import numpy as np


def pinball_loss(y: float, y_hat: float, q: float) -> float:
    """Single observation pinball loss."""
    e = y - y_hat
    return max(q * e, (q - 1.0) * e)


def mean_pinball_loss(
    y: np.ndarray,
    y_hat_q: np.ndarray,
    quantiles: tuple[float, ...],
) -> float:
    """
    y: [N] targets
    y_hat_q: [N, Qn] predicted quantiles aligned with `quantiles`
    """
    y = np.asarray(y, dtype=np.float64).ravel()
    pred = np.asarray(y_hat_q, dtype=np.float64)
    if pred.ndim != 2 or pred.shape[0] != len(y):
        raise ValueError("y_hat_q must be [N, Qn]")
    qn = pred.shape[1]
    if qn != len(quantiles):
        raise ValueError("quantiles length must match y_hat_q second dim")
    total = 0.0
    n = len(y)
    for j, q in enumerate(quantiles):
        for i in range(n):
            total += pinball_loss(float(y[i]), float(pred[i, j]), q)
    return total / max(n * qn, 1)
