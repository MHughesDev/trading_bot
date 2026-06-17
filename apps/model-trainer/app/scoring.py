"""Proper-scoring rules and calibration diagnostics for distributional forecasts.

All functions operate on:
  levels    : sorted list of quantile levels in (0, 1), length L
  predicted : (N, L) float array — predicted quantiles (σ-units or return units, consistent)
  realized  : (N,) float array  — realized outcomes in the same units

References: Gneiting & Raftery (2007), Christoffersen (1998), Diebold & Mariano (1995).
"""

from __future__ import annotations

import math
import typing

import numpy as np
from scipy import stats


# ---------------------------------------------------------------------------
# I-2.3  Proper scoring: CRPS, pinball, log-score
# ---------------------------------------------------------------------------

def pinball_losses(
    levels: list[float],
    predicted: np.ndarray,
    realized: np.ndarray,
) -> np.ndarray:
    """Per-sample mean pinball loss across all levels. Shape (N,)."""
    levels_arr = np.asarray(levels)          # (L,)
    pred = np.asarray(predicted)             # (N, L)
    real = np.asarray(realized)[:, None]     # (N, 1)
    errors = real - pred                     # (N, L)
    pinball = np.where(errors >= 0, levels_arr * errors, (levels_arr - 1.0) * errors)
    return pinball.mean(axis=1)              # (N,)


def crps_from_quantiles(
    levels: list[float],
    predicted: np.ndarray,
    realized: np.ndarray,
) -> dict:
    """CRPS ≈ 2 × mean pinball loss (the quantile-integral representation).

    Returns dict with 'mean', 'per_sample' (list).
    """
    pb = pinball_losses(levels, predicted, realized)
    crps_per_sample = 2.0 * pb
    return {
        "mean": float(crps_per_sample.mean()),
        "per_sample": crps_per_sample.tolist(),
    }


def pinball_metrics(
    levels: list[float],
    predicted: np.ndarray,
    realized: np.ndarray,
) -> dict:
    """Per-level pinball loss + aggregate mean."""
    levels_arr = np.asarray(levels)
    pred = np.asarray(predicted)
    real = np.asarray(realized)[:, None]
    errors = real - pred
    pinball = np.where(errors >= 0, levels_arr * errors, (levels_arr - 1.0) * errors)
    per_level = pinball.mean(axis=0).tolist()
    return {
        "mean": float(np.mean(per_level)),
        "per_level": {str(round(lv, 4)): float(pl) for lv, pl in zip(levels, per_level)},
    }


def log_score(
    levels: list[float],
    predicted: np.ndarray,
    realized: np.ndarray,
) -> dict:
    """Log-score via linear interpolation of the predictive CDF to a density.

    For each sample: estimate density at realized value by finite-differencing
    adjacent quantiles, then take log. Clamp to avoid -inf.
    """
    levels_arr = np.asarray(levels)
    pred = np.asarray(predicted)   # (N, L)
    real = np.asarray(realized)    # (N,)
    n = len(real)

    scores = np.empty(n)
    for i in range(n):
        q = pred[i]          # sorted quantile values
        y = real[i]
        # Interpolate CDF at y via np.interp; extrapolate to [0,1]
        cdf_y = np.interp(y, q, levels_arr, left=0.0, right=1.0)
        # Finite-difference density: Δp / Δq around the bracketing interval
        idx = np.searchsorted(q, y)
        idx = max(1, min(idx, len(q) - 1))
        dp = levels_arr[idx] - levels_arr[idx - 1]
        dq = q[idx] - q[idx - 1]
        if dq <= 0:
            density = 1e-8
        else:
            density = dp / dq
        scores[i] = math.log(max(density, 1e-8))

    return {
        "mean": float(scores.mean()),
        "per_sample": scores.tolist(),
    }


# ---------------------------------------------------------------------------
# I-2.4  Calibration: PIT, coverage, reliability
# ---------------------------------------------------------------------------

def pit_stats(
    levels: list[float],
    predicted: np.ndarray,
    realized: np.ndarray,
    n_bins: int = 10,
) -> dict:
    """Probability Integral Transform (PIT) histogram + KS test for uniformity.

    PIT value for sample i = interpolated CDF value at realized[i].
    Under a calibrated forecaster, PIT ~ Uniform(0, 1).
    """
    levels_arr = np.asarray(levels)
    pred = np.asarray(predicted)
    real = np.asarray(realized)
    n = len(real)

    pit = np.array([
        float(np.interp(real[i], pred[i], levels_arr, left=0.0, right=1.0))
        for i in range(n)
    ])

    # KS test for uniformity
    ks_stat, ks_p = stats.kstest(pit, "uniform")

    # Histogram
    hist, edges = np.histogram(pit, bins=n_bins, range=(0.0, 1.0))
    bin_centers = ((edges[:-1] + edges[1:]) / 2).tolist()

    return {
        "pit_values": pit.tolist(),
        "histogram": {"counts": hist.tolist(), "bin_centers": bin_centers},
        "ks_stat": float(ks_stat),
        "ks_p": float(ks_p),
        "calibrated": bool(ks_p > 0.05),
    }


def interval_coverage(
    levels: list[float],
    predicted: np.ndarray,
    realized: np.ndarray,
) -> dict:
    """Empirical coverage per symmetric quantile pair (lower, upper).

    For each pair (α/2, 1-α/2) from the grid, compute fraction of realized
    values falling inside the predicted interval. Nominal = 1 - α.
    """
    pred = np.asarray(predicted)   # (N, L)
    real = np.asarray(realized)    # (N,)
    levels_arr = np.asarray(levels)

    results = []
    for i, lo_lv in enumerate(levels):
        hi_lv = 1.0 - lo_lv
        if hi_lv <= lo_lv:
            break
        j = int(np.argmin(np.abs(levels_arr - hi_lv)))
        if abs(levels_arr[j] - hi_lv) > 1e-6:
            continue
        nominal = hi_lv - lo_lv
        inside = ((pred[:, i] <= real) & (real <= pred[:, j])).mean()
        results.append({
            "lower_level": float(lo_lv),
            "upper_level": float(levels_arr[j]),
            "nominal": float(nominal),
            "empirical": float(inside),
            "gap": float(inside - nominal),
        })

    return {"coverage": results}


def reliability_diagram(
    levels: list[float],
    predicted: np.ndarray,
    realized: np.ndarray,
) -> dict:
    """Predicted vs empirical frequency data for the reliability diagram.

    For each level α, the empirical frequency = fraction of realized values
    below the predicted α-quantile. Under calibration, empirical ≈ nominal.
    """
    pred = np.asarray(predicted)
    real = np.asarray(realized)

    points = []
    for i, lv in enumerate(levels):
        empirical = float((real <= pred[:, i]).mean())
        points.append({"nominal": float(lv), "empirical": empirical})

    return {"reliability": points}


# ---------------------------------------------------------------------------
# I-2.5  VaR backtests: Kupiec + Christoffersen
# ---------------------------------------------------------------------------

def _exceptions(var_quantile: np.ndarray, realized: np.ndarray) -> np.ndarray:
    """True where realized < VaR (exception = loss exceeds predicted VaR)."""
    return (realized < var_quantile).astype(int)


def kupiec_pof(
    var_quantile: np.ndarray,
    realized: np.ndarray,
    confidence: float = 0.95,
    alpha: float = 0.05,
) -> dict:
    """Kupiec Proportion-of-Failures likelihood-ratio test.

    H0: E[exceptions] = 1 - confidence.
    """
    exc = _exceptions(var_quantile, realized)
    n = len(exc)
    n_exc = int(exc.sum())
    p_hat = n_exc / n if n > 0 else 0.0
    p_null = 1.0 - confidence

    if n_exc == 0 or n_exc == n:
        # Boundary case — LR is numerically problematic; treat as 0
        lr = 0.0
    else:
        lr = -2.0 * (
            n_exc * math.log(p_null / p_hat) + (n - n_exc) * math.log((1 - p_null) / (1 - p_hat))
        )

    p_value = float(stats.chi2.sf(lr, df=1))
    return {
        "n_exceptions": n_exc,
        "expected_exceptions": round(n * p_null, 2),
        "exception_rate": float(p_hat),
        "lr_stat": float(lr),
        "p_value": p_value,
        "passed": p_value > alpha,
        "confidence": confidence,
    }


def christoffersen_cc(
    var_quantile: np.ndarray,
    realized: np.ndarray,
    confidence: float = 0.95,
    alpha: float = 0.05,
) -> dict:
    """Christoffersen conditional-coverage test (independence + POF jointly)."""
    exc = _exceptions(var_quantile, realized).astype(int)
    n = len(exc)
    n_exc = int(exc.sum())
    p_null = 1.0 - confidence

    # Transition counts
    n00 = n01 = n10 = n11 = 0
    for t in range(1, n):
        prev, cur = exc[t - 1], exc[t]
        if prev == 0 and cur == 0:
            n00 += 1
        elif prev == 0 and cur == 1:
            n01 += 1
        elif prev == 1 and cur == 0:
            n10 += 1
        else:
            n11 += 1

    def _safe_log(x: float) -> float:
        return math.log(x) if x > 0 else 0.0

    # Unconditional probability under H0
    p_hat = n_exc / n if n > 0 else 0.0
    p01 = n01 / (n00 + n01) if (n00 + n01) > 0 else 0.0
    p11 = n11 / (n10 + n11) if (n10 + n11) > 0 else 0.0

    # Independence LR
    ll_indep = (
        n00 * _safe_log(1 - p_hat) + n01 * _safe_log(p_hat)
        + n10 * _safe_log(1 - p_hat) + n11 * _safe_log(p_hat)
    )
    ll_dep = (
        n00 * _safe_log(1 - p01) + n01 * _safe_log(p01)
        + n10 * _safe_log(1 - p11) + n11 * _safe_log(p11)
    )
    lr_ind = -2.0 * (ll_indep - ll_dep)

    # Joint CC LR (POF + independence)
    n_no_exc = n - n_exc
    ll_null = n_exc * _safe_log(p_null) + n_no_exc * _safe_log(1 - p_null)
    lr_pof = -2.0 * (ll_null - (n_exc * _safe_log(p_hat) + n_no_exc * _safe_log(1 - p_hat)))
    lr_cc = lr_pof + lr_ind

    p_value_cc = float(stats.chi2.sf(lr_cc, df=2))
    return {
        "n_exceptions": n_exc,
        "exception_rate": float(p_hat),
        "lr_cc_stat": float(lr_cc),
        "p_value_cc": p_value_cc,
        "passed": p_value_cc > alpha,
        "confidence": confidence,
    }


# ---------------------------------------------------------------------------
# I-2.6  Baseline scorecards
# ---------------------------------------------------------------------------

def score_naive_baseline(realized: np.ndarray, levels: list[float]) -> dict:
    """Random-walk baseline: last realized value is the point prediction.

    Distribution: ±σ_hist Normal quantiles centered at 0 (zero-mean random walk).
    """
    if len(realized) < 2:
        return {"baseline": "naive", "crps": {"mean": float("nan")}}
    sigma = float(np.std(np.diff(realized)))
    if sigma <= 0:
        sigma = 1e-8
    q_vals = stats.norm.ppf(levels, loc=0.0, scale=sigma)
    pred = np.tile(q_vals, (len(realized), 1))
    crps = crps_from_quantiles(levels, pred, realized)
    return {"baseline": "naive", "crps": crps}


def score_seasonal_naive_baseline(realized: np.ndarray, levels: list[float], period: int = 1440) -> dict:
    """Seasonal-naive baseline using the same period-of-day return."""
    if len(realized) < period + 1:
        return {"baseline": "seasonal_naive", "crps": {"mean": float("nan")}}
    # Use the value `period` bars ago as center; σ from seasonal residuals
    seasonal_preds = realized[:-period]
    seasonal_realized = realized[period:]
    residuals = seasonal_realized - seasonal_preds
    sigma = float(np.std(residuals))
    if sigma <= 0:
        sigma = 1e-8
    q_offsets = stats.norm.ppf(levels, loc=0.0, scale=sigma)
    pred = seasonal_preds[:, None] + q_offsets[None, :]
    crps = crps_from_quantiles(levels, pred, seasonal_realized)
    return {"baseline": "seasonal_naive", "crps": crps}


def score_zero_shot_baseline(realized: np.ndarray, levels: list[float]) -> dict:
    """Zero-shot baseline: empirical distribution of realized values (non-parametric)."""
    sorted_real = np.sort(realized)
    n = len(sorted_real)
    emp_levels = (np.arange(n) + 0.5) / n
    q_vals = np.interp(levels, emp_levels, sorted_real)
    pred = np.tile(q_vals, (n, 1))
    crps = crps_from_quantiles(levels, pred, realized)
    return {"baseline": "zero_shot_empirical", "crps": crps}


# ---------------------------------------------------------------------------
# I-2.7  Per-fold / per-regime breakdowns
# ---------------------------------------------------------------------------

def per_fold_scores(
    levels: list[float],
    predicted: np.ndarray,
    realized: np.ndarray,
    fold_indices: list[tuple[int, int]],
) -> list[dict]:
    """CRPS per (test_start, test_end) fold slice."""
    results = []
    for fold_i, (start, end) in enumerate(fold_indices):
        pred_fold = predicted[start:end]
        real_fold = realized[start:end]
        if len(real_fold) == 0:
            continue
        crps = crps_from_quantiles(levels, pred_fold, real_fold)
        results.append({"fold": fold_i, "test_start": start, "test_end": end, "crps_mean": crps["mean"]})
    return results


def per_regime_scores(
    levels: list[float],
    predicted: np.ndarray,
    realized: np.ndarray,
    rolling_vol: np.ndarray,
    n_buckets: int = 3,
) -> list[dict]:
    """CRPS by realized-vol regime (low / mid / high tercile)."""
    percentiles = np.percentile(rolling_vol, np.linspace(0, 100, n_buckets + 1))
    results = []
    labels = ["low_vol", "mid_vol", "high_vol"][:n_buckets]
    for i in range(n_buckets):
        lo, hi = percentiles[i], percentiles[i + 1]
        mask = (rolling_vol >= lo) & (rolling_vol < hi)
        if i == n_buckets - 1:
            mask = rolling_vol >= lo
        if mask.sum() == 0:
            continue
        crps = crps_from_quantiles(levels, predicted[mask], realized[mask])
        results.append({
            "regime": labels[i],
            "vol_range": [float(lo), float(hi)],
            "n": int(mask.sum()),
            "crps_mean": crps["mean"],
        })
    return results


# ---------------------------------------------------------------------------
# I-2.8  Overfitting diagnostics
# ---------------------------------------------------------------------------

def deflated_crps(crps_mean: float, n_trials: int, n_test: int) -> float:
    """Bailey & López de Prado deflation: adjust CRPS upward for multiple trials.

    Approximation: deflated = crps * (1 + sqrt(2 * log(n_trials) / n_test)).
    Returns the deflated (pessimistic) CRPS estimate.
    """
    if n_trials <= 1 or n_test <= 0:
        return crps_mean
    penalty = math.sqrt(2.0 * math.log(n_trials) / n_test)
    return crps_mean * (1.0 + penalty)


# ---------------------------------------------------------------------------
# I-2.9  Diebold–Mariano significance test
# ---------------------------------------------------------------------------

def diebold_mariano(losses_a: np.ndarray, losses_b: np.ndarray) -> dict:
    """DM test on the loss differential d_t = L_a(t) - L_b(t).

    H0: E[d] = 0.  Newey-West t-stat for serial correlation.
    Returns stat, p_value, conclusion.
    """
    d = np.asarray(losses_a) - np.asarray(losses_b)
    n = len(d)
    if n < 4:
        return {"dm_stat": float("nan"), "p_value": float("nan"), "significant": False}

    d_mean = d.mean()
    # Newey-West variance with h lags = h_nw
    h_nw = int(math.ceil(n ** (1.0 / 3.0)))
    gamma0 = float(np.var(d, ddof=0))
    nw_var = gamma0
    for lag in range(1, h_nw + 1):
        gamma_lag = float(np.cov(d[:-lag], d[lag:])[0, 1]) if lag < n else 0.0
        nw_var += 2.0 * (1.0 - lag / (h_nw + 1.0)) * gamma_lag
    se = math.sqrt(max(nw_var / n, 1e-20))
    dm_stat = d_mean / se
    p_value = float(2.0 * stats.norm.sf(abs(dm_stat)))
    return {
        "dm_stat": float(dm_stat),
        "p_value": p_value,
        "significant": p_value < 0.05,
        "better": "a" if dm_stat < 0 else "b",
    }


# ---------------------------------------------------------------------------
# Aggregate eval runner
# ---------------------------------------------------------------------------

def evaluate_distribution(
    levels: list[float],
    predicted: np.ndarray,
    realized: np.ndarray,
    trial_count: int = 0,
    fold_test_ranges: list[tuple[int, int]] | None = None,
    rolling_vol: np.ndarray | None = None,
) -> dict:
    """Full scoring suite for one (levels, predicted, realized) triple.

    Returns a flat metrics dict suitable for JSON persistence.
    """
    n = len(realized)
    if n == 0:
        return {"error": "empty eval set"}

    crps = crps_from_quantiles(levels, predicted, realized)
    pinball = pinball_metrics(levels, predicted, realized)
    log_sc = log_score(levels, predicted, realized)
    pit = pit_stats(levels, predicted, realized)
    cov = interval_coverage(levels, predicted, realized)
    rel = reliability_diagram(levels, predicted, realized)

    # VaR backtests at the lowest configured level (tail risk)
    tail_idx = 0  # e.g. 0.05-quantile = 5% VaR
    var_q = predicted[:, tail_idx]
    kup = kupiec_pof(var_q, realized, confidence=1.0 - levels[tail_idx])
    chris = christoffersen_cc(var_q, realized, confidence=1.0 - levels[tail_idx])

    # Overfitting diagnostics
    defl = deflated_crps(crps["mean"], trial_count, n)

    # Per-fold
    fold_scores: list = []
    if fold_test_ranges:
        fold_scores = per_fold_scores(levels, predicted, realized, fold_test_ranges)

    # Per-regime
    regime_scores: list = []
    if rolling_vol is not None and len(rolling_vol) == n:
        regime_scores = per_regime_scores(levels, predicted, realized, rolling_vol)

    # Baselines
    naive_bl = score_naive_baseline(realized, levels)
    zero_bl = score_zero_shot_baseline(realized, levels)
    # DM vs naive
    naive_pred_q = np.tile(
        stats.norm.ppf(levels, loc=0.0, scale=float(np.std(np.diff(realized)) or 1e-8)),
        (n, 1),
    )
    losses_model = pinball_losses(levels, predicted, realized)
    losses_naive = pinball_losses(levels, naive_pred_q, realized)
    dm_naive = diebold_mariano(losses_model, losses_naive)

    return {
        "n": n,
        "crps": crps["mean"],
        "crps_deflated": defl,
        "trial_count": trial_count,
        "pinball": pinball,
        "log_score": log_sc["mean"],
        "pit": {
            "ks_stat": pit["ks_stat"],
            "ks_p": pit["ks_p"],
            "calibrated": pit["calibrated"],
            "histogram": pit["histogram"],
        },
        "coverage": cov["coverage"],
        "reliability": rel["reliability"],
        "var_backtest": {
            "kupiec": kup,
            "christoffersen": chris,
        },
        "overfitting": {
            "trial_count": trial_count,
            "crps_raw": crps["mean"],
            "crps_deflated": defl,
        },
        "baselines": {
            "naive": naive_bl,
            "zero_shot": zero_bl,
        },
        "beats_naive": bool(crps["mean"] < naive_bl["crps"].get("mean", float("inf"))),
        "dm_vs_naive": dm_naive,
        "per_fold": fold_scores,
        "per_regime": regime_scores,
    }
