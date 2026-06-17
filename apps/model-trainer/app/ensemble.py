"""Ensemble combination, conformal calibration, and quantile-crossing repair.

Implements ADR-0018 (Phase 4):
  - Linear opinion pool (I-4.4)
  - CRPS-weighted adaptive combiner (I-4.5)
  - Stacking combiner — cal-only meta-learner (I-4.6)
  - Weight floors & temperature (I-4.7)
  - Spine-as-coordinate-setter, σ-combine + rescale (I-4.8)
  - Adaptive conformal calibration wrapper (I-4.9)
  - Quantile-crossing repair (I-4.10)

All combiners operate on (N, L) predicted-quantile arrays (σ-units).
Realized labels are (N,) arrays in matching units.
"""

from __future__ import annotations

import math
from typing import TYPE_CHECKING

import numpy as np
from scipy.optimize import nnls

if TYPE_CHECKING:
    pass


# ---------------------------------------------------------------------------
# I-4.7 Weight floors & temperature helpers
# ---------------------------------------------------------------------------

def _apply_temperature(raw_weights: np.ndarray, temperature: float) -> np.ndarray:
    """Sharpen (T<1) or soften (T>1) raw weights via exponent 1/T, then normalize."""
    if abs(temperature - 1.0) < 1e-9:
        return raw_weights / raw_weights.sum()
    w = raw_weights ** (1.0 / temperature)
    s = w.sum()
    return w / s if s > 0 else np.full_like(raw_weights, 1.0 / len(raw_weights))


def _apply_weight_floor(weights: np.ndarray, floor: float) -> np.ndarray:
    """Project weights onto [floor, 1] simplex iteratively.

    Each member receives at least `floor`; remaining mass is distributed
    proportionally among members that are above the floor.
    """
    n = len(weights)
    w = weights.copy()
    for _ in range(n + 1):
        low = w < floor
        if not low.any():
            break
        deficit = (floor - w[low]).sum()
        w[low] = floor
        above = ~low
        if not above.any():
            break
        w[above] -= deficit * w[above] / w[above].sum()
    # Renormalize for floating-point safety.
    return w / w.sum()


# ---------------------------------------------------------------------------
# I-4.8 Spine-as-coordinate-setter: project members onto σ_spine, rescale output
# ---------------------------------------------------------------------------

def project_to_spine(
    member_quantiles: list[np.ndarray],  # list of (N, L) arrays, each in member's σ-units
    member_sigmas: list[float],          # per-member σ scaler (from bundle header)
    sigma_spine: float,                  # spine σ (first member's or explicit)
) -> list[np.ndarray]:
    """Re-scale every member's quantile array to the spine's σ-coordinate."""
    projected = []
    for q, sigma in zip(member_quantiles, member_sigmas):
        if abs(sigma) < 1e-12 or abs(sigma_spine) < 1e-12:
            projected.append(q)
        else:
            projected.append(q * (sigma_spine / sigma))
    return projected


def rescale_from_spine(combined: np.ndarray, sigma_spine: float) -> np.ndarray:
    """Convert combined σ-unit quantiles back to return units."""
    return combined * sigma_spine


# ---------------------------------------------------------------------------
# I-4.4 Linear opinion pool
# ---------------------------------------------------------------------------

def linear_opinion_pool(
    member_quantiles: list[np.ndarray],  # list of (N, L); already in σ-units (spine-projected)
    weights: np.ndarray,                 # (M,) normalized weights
) -> np.ndarray:
    """Weighted average of member quantile functions — the linear opinion pool.

    Returns (N, L) combined quantile array.
    """
    combined = np.zeros_like(member_quantiles[0])
    for q, w in zip(member_quantiles, weights):
        combined += w * q
    return combined


# ---------------------------------------------------------------------------
# I-4.5 CRPS-weighted adaptive combiner
# ---------------------------------------------------------------------------

def crps_weights(
    member_crps: list[float],  # per-member CRPS (lower = better)
    weight_floor: float,
    temperature: float,
) -> np.ndarray:
    """Compute CRPS-weighted member weights.

    w_raw[i] = 1 / crps[i]  (inverse-CRPS; better member → higher weight).
    Then apply temperature + floor.
    """
    inv = np.array([1.0 / max(c, 1e-12) for c in member_crps])
    raw = _apply_temperature(inv, temperature)
    return _apply_weight_floor(raw, weight_floor)


def crps_weighted_pool(
    member_quantiles: list[np.ndarray],
    member_crps: list[float],
    weight_floor: float,
    temperature: float,
) -> tuple[np.ndarray, np.ndarray]:
    """CRPS-weighted linear opinion pool.

    Returns (combined (N,L), weights (M,)).
    """
    w = crps_weights(member_crps, weight_floor, temperature)
    return linear_opinion_pool(member_quantiles, w), w


# ---------------------------------------------------------------------------
# I-4.6 Stacking combiner — cal-role only meta-learner
# ---------------------------------------------------------------------------

def stacking_fit(
    member_cal_quantiles: list[np.ndarray],  # each (N_cal, L), spine-projected
    cal_realized: np.ndarray,                # (N_cal,) realized outcomes
    levels: list[float],
) -> dict:
    """Fit stacking meta-learner on calibration data only (never test).

    Uses non-negative least squares per quantile level to find mixing weights.
    Leakage invariant: caller must pass ONLY calibration-role rows.

    Returns a state dict with `level_weights` (L, M).
    """
    M = len(member_cal_quantiles)
    L = len(levels)
    level_weights = np.zeros((L, M))

    for li in range(L):
        # Build (N_cal, M) design matrix of member quantile predictions at level li.
        X = np.column_stack([q[:, li] for q in member_cal_quantiles])
        y = cal_realized
        try:
            w, _ = nnls(X, y)
        except Exception:
            w = np.ones(M) / M
        s = w.sum()
        level_weights[li] = w / s if s > 1e-12 else np.ones(M) / M

    return {"level_weights": level_weights.tolist()}


def stacking_predict(
    member_quantiles: list[np.ndarray],  # each (N, L), spine-projected
    state: dict,
) -> np.ndarray:
    """Apply fitted stacking weights to produce combined (N, L) quantiles."""
    level_weights = np.array(state["level_weights"])  # (L, M)
    L, M = level_weights.shape
    N = member_quantiles[0].shape[0]
    combined = np.zeros((N, L))
    for li in range(L):
        for mi, q in enumerate(member_quantiles):
            combined[:, li] += level_weights[li, mi] * q[:, li]
    return combined


# ---------------------------------------------------------------------------
# I-4.9 Adaptive conformal calibration wrapper
# ---------------------------------------------------------------------------

def conformal_fit(
    predicted_cal: np.ndarray,  # (N_cal, L) combined quantiles (σ-units)
    realized_cal: np.ndarray,   # (N_cal,) realized outcomes
    levels: list[float],
    alpha: float = 0.05,        # ACI step size
) -> dict:
    """Fit adaptive conformal calibration residuals on the calibration role.

    For each level τ, stores nonconformity scores |y - q̂_τ| and computes
    an initial adjustment.  The adaptive variant (ACI) also stores the step
    size for online updates.

    Returns a state dict with `scores`, `adjustments`, `alpha`.
    """
    N_cal, L = predicted_cal.shape
    scores = []
    adjustments = []

    for li, tau in enumerate(levels):
        q_col = predicted_cal[:, li]
        # Signed nonconformity score: positive means y was above the quantile.
        nc = realized_cal - q_col
        scores.append(nc.tolist())
        # Adjustment = (1 - tau) quantile of |nc| → ensures ~tau empirical coverage.
        level_idx = math.ceil((1 - tau) * N_cal)
        level_idx = min(max(level_idx, 0), N_cal - 1)
        adj = float(np.sort(np.abs(nc))[level_idx])
        adjustments.append(adj)

    return {
        "scores": scores,        # (L, N_cal)
        "adjustments": adjustments,  # (L,) — added to quantile at inference
        "alpha": alpha,
        "levels": levels,
    }


def conformal_update(
    state: dict,
    new_realized: float,
    new_predicted: list[float],  # length L
) -> dict:
    """Online ACI update: incorporate one new realized outcome.

    Shifts each adjustment toward the empirical coverage of the new observation.
    """
    import copy
    state = copy.deepcopy(state)
    alpha = state["alpha"]
    levels = state["levels"]

    for li, tau in enumerate(levels):
        q_hat = new_predicted[li]
        # 1 if covered (y ≤ q̂ + adj), else 0; gradient step on adjustment.
        covered = float(new_realized <= q_hat + state["adjustments"][li])
        # ACI: adjustment += alpha * (tau - covered)
        state["adjustments"][li] += alpha * (tau - covered)

    return state


def conformal_predict(
    predicted: np.ndarray,  # (N, L)
    state: dict,
) -> np.ndarray:
    """Apply conformal adjustments to combined quantile predictions.

    Shifts each quantile column by its fitted adjustment.
    """
    adjustments = np.array(state["adjustments"])  # (L,)
    return predicted + adjustments[np.newaxis, :]


# ---------------------------------------------------------------------------
# I-4.10 Quantile-crossing repair (inherited from Phase 1 — monotone enforcement)
# ---------------------------------------------------------------------------

def repair_crossings(predicted: np.ndarray) -> tuple[np.ndarray, int]:
    """Enforce monotone quantile order via row-wise sort.

    Returns (repaired (N, L), crossing_count).
    """
    sorted_pred = np.sort(predicted, axis=1)
    crossings = int(np.sum(sorted_pred != predicted))
    return sorted_pred, crossings


# ---------------------------------------------------------------------------
# I-4.4 / I-4.11 Top-level combine() entry point called by the sidecar worker
# ---------------------------------------------------------------------------

def combine(
    member_quantiles: list[np.ndarray],  # list of (N, L) — in each member's σ-units
    member_sigmas: list[float],
    realized: np.ndarray,               # (N,) — in spine return units
    levels: list[float],
    combiner: str,
    weight_floor: float,
    temperature: float,
    cal_mask: np.ndarray | None = None,  # bool mask — calibration rows
    member_crps: list[float] | None = None,
    stacking_state: dict | None = None,
    conformal_state: dict | None = None,
) -> dict:
    """End-to-end ensemble combine pipeline.

    Returns a result dict with:
      `combined`       : (N, L) return-unit combined quantiles
      `weights`        : (M,) final member weights (None for stacking)
      `crossing_count` : int
      `conformal_state`: updated conformal state (for persistence in bundle)
      `stacking_state` : fitted stacking state (for persistence in bundle)
    """
    sigma_spine = member_sigmas[0] if member_sigmas else 1.0

    # Project all members to spine σ-coordinates.
    projected = project_to_spine(member_quantiles, member_sigmas, sigma_spine)

    weights = None
    new_stacking_state = stacking_state

    if combiner == "linear_opinion_pool":
        uniform = np.ones(len(projected)) / len(projected)
        raw = _apply_temperature(uniform, temperature)
        w = _apply_weight_floor(raw, weight_floor)
        combined_sigma = linear_opinion_pool(projected, w)
        weights = w

    elif combiner == "crps_weighted":
        if member_crps is None:
            member_crps = [1.0] * len(projected)
        combined_sigma, weights = crps_weighted_pool(
            projected, member_crps, weight_floor, temperature
        )

    elif combiner == "stacking":
        if cal_mask is not None and cal_mask.any():
            cal_proj = [q[cal_mask] for q in projected]
            cal_real = realized[cal_mask]
            new_stacking_state = stacking_fit(cal_proj, cal_real, levels)
        if new_stacking_state is None:
            # Fallback: uniform LOP if no cal data yet.
            w = np.ones(len(projected)) / len(projected)
            combined_sigma = linear_opinion_pool(projected, w)
        else:
            combined_sigma = stacking_predict(projected, new_stacking_state)

    else:
        raise ValueError(f"unknown combiner: {combiner!r}")

    # Rescale from σ-units to return units.
    combined_return = rescale_from_spine(combined_sigma, sigma_spine)

    # Apply adaptive conformal calibration.
    new_conf_state = conformal_state
    if conformal_state is not None:
        combined_return = conformal_predict(combined_return, conformal_state)

    # Fit/update conformal on calibration rows (if cal data available).
    if cal_mask is not None and cal_mask.any():
        cal_combined = combined_return[cal_mask]
        cal_real = realized[cal_mask]
        if conformal_state is None:
            new_conf_state = conformal_fit(cal_combined, cal_real, levels)
        else:
            # Online update for each cal row.
            for i in range(cal_combined.shape[0]):
                new_conf_state = conformal_update(
                    new_conf_state,
                    float(cal_real[i]),
                    cal_combined[i].tolist(),
                )

    # Quantile-crossing repair.
    combined_return, crossing_count = repair_crossings(combined_return)

    return {
        "combined": combined_return,
        "weights": weights.tolist() if weights is not None else None,
        "crossing_count": crossing_count,
        "conformal_state": new_conf_state,
        "stacking_state": new_stacking_state,
    }
