"""Adaptive conformal calibration (Set N, N-4.1; FEAT-006 section 8).

Wraps any model's point (or quantile-band) predictions into calibrated intervals
and reports the coverage it actually delivered through time.

Methods
-------
``split``      static split conformal: the quantile of the first `n_cal` absolute
               residuals, frozen. The thing the others must beat on a regime shift.
``aci``        adaptive conformal inference (Gibbs and Candes 2021). The
               miscoverage level moves after every outcome,
               ``alpha_{t+1} = alpha_t + gamma * (alpha - err_t)`` with
               ``err_t = 1{y_t outside the band}``, and the band uses the
               ``1 - alpha_t`` quantile of a rolling window of past scores.
``aci_cusum``  ``aci`` plus a two-sided CUSUM on the coverage-error stream
               ``x_t = err_t - alpha``: ``g+ = max(0, g+ + x - k)``,
               ``g- = max(0, g- - x - k)``; it fires when either exceeds `h`, then
               both restart at zero. The constants are the platform monitor's
               (``crates/model-registry/src/calibration.rs``: k = 0.05, h = 4.0),
               so the alert and this reset agree. On a fire the adaptive state is
               reset: ``alpha_t`` returns to the target alpha and the score window is
               cut to the observations since the change point (the start of the CUSUM
               excursion that fired), keeping at least `min_window` of the most recent
               scores so the new quantile is not read off two or three points.

Scores
------
With only point predictions the score is ``|y - point|`` and the band is
``point +/- qhat``. With ``lower`` / ``upper`` (a model's own interval, e.g. its 5%
and 95% quantiles) the score is the conformalised-quantile-regression score
``max(lower - y, y - upper)`` and the band is ``[lower - qhat, upper + qhat]``.
`spread_90` is the band's width, ``upper - lower``, the convention sizing consumes
when alpha = 0.10.

The first `n_cal` observations only calibrate: they produce no band and are not in
any coverage figure. `calibrate` is what `POST /calibrate` runs.
"""

from __future__ import annotations

import math
from typing import Optional

import numpy as np
from pydantic import BaseModel, Field

from . import failures

DEFAULT_K = 0.05
DEFAULT_H = 4.0
METHODS = ("split", "aci", "aci_cusum")


class CalibrateRequest(BaseModel):
    scores: list[float] = Field(description="point predictions, one per time step")
    realized: list[float]
    lower: Optional[list[float]] = None
    upper: Optional[list[float]] = None
    method: str = "aci_cusum"
    alpha: float = 0.10
    gamma: float = 0.005
    n_cal: int = 100
    window: int = 200
    rolling: int = 100
    k: float = DEFAULT_K
    h: float = DEFAULT_H
    min_window: int = 20


def _conformal_quantile(scores: np.ndarray, level: float) -> float:
    """Finite-sample conformal quantile: the ceil((n+1)*level)-th smallest score,
    clipped to the largest score when that rank passes n."""
    n = len(scores)
    if n == 0:
        return float("nan")
    level = min(max(level, 0.0), 1.0)
    rank = math.ceil((n + 1) * level)
    s = np.sort(scores)
    return float(s[min(max(rank, 1), n) - 1])


def _nan_to_none(a) -> list:
    return [None if not np.isfinite(v) else float(v) for v in a]


def calibrate(
    scores,
    realized,
    *,
    lower=None,
    upper=None,
    method: str = "aci_cusum",
    alpha: float = 0.10,
    gamma: float = 0.005,
    n_cal: int = 100,
    window: int = 200,
    rolling: int = 100,
    k: float = DEFAULT_K,
    h: float = DEFAULT_H,
    min_window: int = 20,
) -> dict:
    """Run the wrapper over a sequence of (prediction, outcome) pairs, in time order."""
    if method not in METHODS:
        raise failures.DataError(f"unknown calibration method {method!r}; use one of {list(METHODS)}")
    if not (0.0 < alpha < 1.0):
        raise failures.DataError(f"alpha must be in (0, 1), got {alpha}")
    if gamma < 0 or n_cal < 2 or window < 2 or rolling < 1 or min_window < 1:
        raise failures.DataError("gamma >= 0, n_cal >= 2, window >= 2, rolling >= 1, min_window >= 1 required")
    point = np.asarray(scores, dtype=float)
    y = np.asarray(realized, dtype=float)
    n = len(y)
    if len(point) != n:
        raise failures.DataError(f"scores ({len(point)}) and realized ({n}) differ in length")
    if (lower is None) != (upper is None):
        raise failures.DataError("lower and upper must be given together")
    if lower is not None:
        lo_in, hi_in = np.asarray(lower, dtype=float), np.asarray(upper, dtype=float)
        if len(lo_in) != n or len(hi_in) != n:
            raise failures.DataError("lower/upper must match realized in length")
    else:
        lo_in, hi_in = point, point
    if n <= n_cal:
        raise failures.DataError(f"need more than n_cal={n_cal} observations, got {n}")
    if not (np.isfinite(point).all() and np.isfinite(y).all() and np.isfinite(lo_in).all() and np.isfinite(hi_in).all()):
        raise failures.DataError("non-finite values in scores / realized / band")

    nonconf = np.maximum(lo_in - y, y - hi_in) if lower is not None else np.abs(y - point)

    lower_out = np.full(n, np.nan)
    upper_out = np.full(n, np.nan)
    qhat_out = np.full(n, np.nan)
    alpha_t_out = np.full(n, np.nan)
    err = np.full(n, np.nan)
    resets: list[int] = []

    static_q = _conformal_quantile(nonconf[:n_cal], 1.0 - alpha)
    adaptive = method in ("aci", "aci_cusum")
    cusum = method == "aci_cusum"
    alpha_t = alpha
    win_start = max(0, n_cal - window)  # index of the oldest score in the window
    g_pos = g_neg = 0.0
    start_pos = start_neg = None  # where the current excursion left zero

    for t in range(n_cal, n):
        if adaptive:
            lo_i = max(win_start, t - window)
            q = _conformal_quantile(nonconf[lo_i:t], 1.0 - alpha_t)
        else:
            q = static_q
        qhat_out[t] = q
        alpha_t_out[t] = alpha_t
        lower_out[t] = lo_in[t] - q
        upper_out[t] = hi_in[t] + q
        miss = 0.0 if (lower_out[t] <= y[t] <= upper_out[t]) else 1.0
        err[t] = miss
        if not adaptive:
            continue
        alpha_t = alpha_t + gamma * (alpha - miss)
        if cusum:
            x = miss - alpha
            new_pos = max(0.0, g_pos + x - k)
            new_neg = max(0.0, g_neg - x - k)
            if g_pos == 0.0 and new_pos > 0.0:
                start_pos = t
            if g_neg == 0.0 and new_neg > 0.0:
                start_neg = t
            g_pos, g_neg = new_pos, new_neg
            if g_pos > h or g_neg > h:
                start = start_pos if g_pos > h else start_neg
                resets.append(t)
                g_pos = g_neg = 0.0
                start_pos = start_neg = None
                alpha_t = alpha
                # the window becomes "since the change point", never fewer than min_window scores
                win_start = max(0, min(int(start if start is not None else t), t + 1 - min_window))

    covered = 1.0 - err
    valid = np.isfinite(err)
    roll = np.full(n, np.nan)
    for t in range(n_cal, n):
        lo_i = max(n_cal, t - rolling + 1)
        roll[t] = covered[lo_i : t + 1].mean()
    spread = upper_out - lower_out

    return {
        "method": method,
        "alpha": alpha,
        "nominal_coverage": 1.0 - alpha,
        "gamma": gamma,
        "k": k,
        "h": h,
        "n": n,
        "n_cal": n_cal,
        "window": window,
        "rolling_window": rolling,
        "lower": _nan_to_none(lower_out),
        "upper": _nan_to_none(upper_out),
        "spread_90": _nan_to_none(spread),
        "qhat": _nan_to_none(qhat_out),
        "alpha_t": _nan_to_none(alpha_t_out),
        "covered": [None if not v else bool(c == 1.0) for v, c in zip(valid, np.nan_to_num(covered))],
        "rolling_coverage": _nan_to_none(roll),
        "coverage": float(covered[valid].mean()),
        "reset_indices": resets,
    }


def mean_abs_coverage_gap(result: dict, start: int = 0) -> float:
    """Mean |rolling coverage - nominal| from index `start` on: how far a run sat from nominal."""
    roll = [v for v in result["rolling_coverage"][start:] if v is not None]
    return float(np.mean([abs(v - result["nominal_coverage"]) for v in roll])) if roll else float("nan")


def run(req: CalibrateRequest) -> dict:
    return calibrate(
        req.scores, req.realized, lower=req.lower, upper=req.upper, method=req.method, alpha=req.alpha,
        gamma=req.gamma, n_cal=req.n_cal, window=req.window, rolling=req.rolling, k=req.k, h=req.h,
        min_window=req.min_window,
    )
