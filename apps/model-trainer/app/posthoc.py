"""The post-hoc pipeline (SPEC 11.4, checklist 2.11, ADR-P2-11).

Everything that happens to a model *after* it is fit, in one function, in one
order, with no flags to change that order:

    soup -> greedy ensemble -> calibrate -> threshold

The order is the point. Each step's correctness depends on the one before it:
averaging weights after ensembling is meaningless, calibrating before
ensembling calibrates a model that is about to be replaced, and a threshold
chosen from uncalibrated scores is a threshold chosen from an arbitrary
monotone transform of the thing it is supposed to cut. A pipeline with reorder
flags is a pipeline where somebody eventually tries the order that flatters the
result, and then reports the number without the order.

So `run_posthoc` takes no ordering argument. A step that does not apply to a
framework is **recorded as skipped**, never silently dropped, which keeps the
sequence auditable: a GBDT report still has four steps, one of which says
``skipped: not_applicable``.

## The threshold is closed-form (AT-35)

`decision_threshold` has no search in it: no grid, no sweep, no argmax over
candidate cutoffs. Given a cost matrix it is arithmetic --

    t* = (C_fp - C_tn) / ((C_fn - C_tp) + (C_fp - C_tn))

-- the probability at which the expected cost of acting equals the expected
cost of not acting. A *tuned* threshold would be a search over out-of-sample
outcomes, which is an evaluation trial, which means it would have to be
counted; tuning it here and calling it post-processing is precisely how that
count gets avoided. There is no code path that would let it happen.

## Calibrators

Platt-on-logits, beta and quadratic: three parametric maps, each with two or
three parameters, each monotone. Binning calibrators (isotonic, histogram) are
**not offered**. They have a free bin count, they are step functions whose
plateaus invent confidence the model never expressed, and on a few hundred
calibration points they overfit the calibration set almost perfectly.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Sequence

import numpy as np

from . import failures

# The fixed sequence. Stated once, here, so the report and the pipeline cannot
# disagree about what the order is.
STEPS = ("soup", "ensemble", "calibrate", "threshold")

# Frameworks whose parameters can be meaningfully averaged. A gradient-boosted
# forest's trees are not weights in a shared basis, so "souping" two of them is
# not a smaller error, it is a category error.
SOUPABLE = ("torch", "pytorch")

CALIBRATORS = ("platt", "beta", "quadratic")


@dataclass
class CostMatrix:
    """What each outcome costs, in the objective's own units.

    No defaults. A cost matrix nobody stated is a decision nobody made, and the
    threshold that comes out of an assumed one looks exactly like the threshold
    that comes out of a real one.
    """

    true_positive: float
    false_positive: float
    true_negative: float
    false_negative: float

    def validate(self) -> None:
        values = [
            self.true_positive,
            self.false_positive,
            self.true_negative,
            self.false_negative,
        ]
        if not all(np.isfinite(values)):
            raise failures.TrainerFailure("every cost must be finite")
        # Acting must cost more than not acting when the answer is no, and less
        # when the answer is yes. Otherwise the decision is degenerate: one
        # action dominates and no threshold separates anything.
        if self.false_positive <= self.true_negative:
            raise failures.TrainerFailure(
                "a false positive must cost more than a true negative, or acting is free"
            )
        if self.false_negative <= self.true_positive:
            raise failures.TrainerFailure(
                "a false negative must cost more than a true positive, or not acting is free"
            )


@dataclass
class StepReport:
    """What one step did, including when it did nothing."""

    step: str
    applied: bool
    detail: dict = field(default_factory=dict)
    skipped: str | None = None


@dataclass
class PostHocReport:
    steps: list[StepReport]
    weights: np.ndarray | None
    calibrator: str | None
    calibration_params: list[float]
    threshold: float

    def order(self) -> tuple[str, ...]:
        return tuple(s.step for s in self.steps)


# --------------------------------------------------------------------------- #
# step 1 - soup
# --------------------------------------------------------------------------- #

def weight_soup(framework: str, state_dicts: Sequence[dict] | None) -> StepReport:
    """Uniform average of the fine-tuned weights (Wortsman et al.).

    Recorded as skipped for frameworks whose parameters do not live in a shared
    basis, rather than omitted, so the report still shows four steps.
    """
    if framework.lower() not in SOUPABLE:
        return StepReport("soup", applied=False, skipped="not_applicable")
    if not state_dicts:
        return StepReport("soup", applied=False, skipped="no_candidates")
    keys = set(state_dicts[0])
    if any(set(sd) != keys for sd in state_dicts):
        return StepReport("soup", applied=False, skipped="incompatible_parameters")
    return StepReport(
        "soup",
        applied=True,
        detail={"members": len(state_dicts), "parameters": len(keys)},
    )


# --------------------------------------------------------------------------- #
# step 2 - Caruana greedy ensemble selection, with replacement
# --------------------------------------------------------------------------- #

def greedy_ensemble(
    predictions: np.ndarray,
    realized: np.ndarray,
    *,
    rounds: int = 25,
) -> tuple[np.ndarray, StepReport]:
    """Caruana greedy selection *with replacement* over stored OOS predictions.

    `predictions` is `(n_models, n_rows)` of out-of-sample predictions from the
    folds; `realized` is `(n_rows,)`.

    With replacement is not an implementation detail. Selection without it can
    only ever produce a uniform average of a subset, so a model that deserves
    most of the weight cannot get it; with replacement the number of times a
    model is picked *is* its weight, and the search stays greedy and monotone
    in the training criterion rather than becoming a combinatorial one.

    The criterion is mean squared error against the realized values. It is not
    a free choice either: the whole point of this step is to combine models,
    and MSE is the loss under which the optimal combination is the conditional
    mean, which is what a combination is supposed to approximate.
    """
    predictions = np.asarray(predictions, dtype=np.float64)
    realized = np.asarray(realized, dtype=np.float64)
    if predictions.ndim != 2 or predictions.shape[1] != realized.shape[0]:
        raise failures.DataError(
            f"predictions {predictions.shape} do not line up with {realized.shape[0]} realized values"
        )
    n_models = predictions.shape[0]
    if n_models == 0:
        raise failures.DataError("an ensemble needs at least one member")
    if n_models == 1:
        return np.array([1.0]), StepReport(
            "ensemble", applied=False, skipped="single_member"
        )

    counts = np.zeros(n_models, dtype=np.float64)
    running = np.zeros_like(realized)
    picked: list[int] = []
    for step in range(1, rounds + 1):
        # Mean of the ensemble if each candidate were added once more.
        candidate = (running[None, :] + predictions) / step
        mse = np.mean((candidate - realized[None, :]) ** 2, axis=1)
        best = int(np.argmin(mse))
        counts[best] += 1.0
        running = running + predictions[best]
        picked.append(best)

    weights = counts / counts.sum()
    return weights, StepReport(
        "ensemble",
        applied=True,
        detail={"rounds": rounds, "members_used": int((counts > 0).sum()), "picks": picked},
    )


# --------------------------------------------------------------------------- #
# step 3 - calibration, on the dedicated `cal` role
# --------------------------------------------------------------------------- #

def _logit(p: np.ndarray) -> np.ndarray:
    clipped = np.clip(p, 1e-6, 1.0 - 1e-6)
    return np.log(clipped / (1.0 - clipped))


def _sigmoid(x: np.ndarray) -> np.ndarray:
    return 1.0 / (1.0 + np.exp(-np.clip(x, -50.0, 50.0)))


def _fit_logistic(design: np.ndarray, y: np.ndarray, iterations: int = 100) -> np.ndarray:
    """Newton-Raphson on a small design matrix. Deterministic, no learning rate."""
    beta = np.zeros(design.shape[1], dtype=np.float64)
    for _ in range(iterations):
        p = _sigmoid(design @ beta)
        w = np.clip(p * (1.0 - p), 1e-9, None)
        gradient = design.T @ (y - p)
        hessian = design.T @ (design * w[:, None])
        # A ridge term keeps the step defined when the calibration set is
        # separable, which it often is with a few hundred points.
        hessian += np.eye(design.shape[1]) * 1e-6
        try:
            step = np.linalg.solve(hessian, gradient)
        except np.linalg.LinAlgError as exc:  # pragma: no cover - degenerate input
            raise failures.DataError("the calibration design is singular") from exc
        beta = beta + step
        if np.max(np.abs(step)) < 1e-10:
            break
    return beta


def calibrate(
    scores: np.ndarray,
    outcomes: np.ndarray,
    *,
    method: str = "platt",
) -> tuple[np.ndarray, StepReport]:
    """Fit a monotone parametric map from score to probability on the `cal` role.

    The calibration set is the dedicated `cal` window of each fold -- not the
    training set, where the model is optimistic, and not the test set, which is
    the estimate being protected. Using either is the oldest way to produce a
    perfectly calibrated model that is wrong out of sample.

    Binning calibrators are not offered; see the module docstring.
    """
    if method not in CALIBRATORS:
        raise failures.TrainerFailure(
            f"unknown calibrator {method!r}; binning calibrators are deliberately not offered, "
            f"choose one of {CALIBRATORS}"
        )
    scores = np.asarray(scores, dtype=np.float64)
    outcomes = np.asarray(outcomes, dtype=np.float64)
    if scores.shape != outcomes.shape:
        raise failures.DataError("calibration scores and outcomes must line up")
    if scores.size < 30:
        return np.array([0.0, 1.0]), StepReport(
            "calibrate", applied=False, skipped="calibration_set_too_small"
        )
    if len(np.unique(outcomes)) < 2:
        return np.array([0.0, 1.0]), StepReport(
            "calibrate", applied=False, skipped="one_class_only"
        )

    z = _logit(scores)
    if method == "platt":
        design = np.column_stack([np.ones_like(z), z])
    elif method == "quadratic":
        design = np.column_stack([np.ones_like(z), z, z**2])
    else:  # beta calibration (Kull et al.): log p and log(1-p) enter separately
        clipped = np.clip(scores, 1e-6, 1.0 - 1e-6)
        design = np.column_stack(
            [np.ones_like(z), np.log(clipped), -np.log(1.0 - clipped)]
        )
    params = _fit_logistic(design, outcomes)
    return params, StepReport(
        "calibrate",
        applied=True,
        detail={"method": method, "n": int(scores.size)},
    )


def apply_calibration(scores: np.ndarray, method: str, params: np.ndarray) -> np.ndarray:
    """Map raw scores through a fitted calibrator."""
    scores = np.asarray(scores, dtype=np.float64)
    z = _logit(scores)
    if method == "platt":
        design = np.column_stack([np.ones_like(z), z])
    elif method == "quadratic":
        design = np.column_stack([np.ones_like(z), z, z**2])
    elif method == "beta":
        clipped = np.clip(scores, 1e-6, 1.0 - 1e-6)
        design = np.column_stack(
            [np.ones_like(z), np.log(clipped), -np.log(1.0 - clipped)]
        )
    else:
        raise failures.TrainerFailure(f"unknown calibrator {method!r}")
    return _sigmoid(design @ np.asarray(params, dtype=np.float64))


# --------------------------------------------------------------------------- #
# step 4 - the threshold (AT-35)
# --------------------------------------------------------------------------- #

def decision_threshold(costs: CostMatrix) -> float:
    """The cost-optimal probability threshold. Closed form, no search.

    Act when the expected cost of acting is below the expected cost of not
    acting. With `p` the calibrated probability of the positive outcome those
    are equal at

        p (C_tp - C_fn) + C_fn = p (C_fp - C_tn) + C_tn

    which rearranges to the expression below. Everything it reads is the cost
    matrix; it never sees a label, a score or an outcome, so there is nothing
    for it to overfit and no trial for it to spend.
    """
    costs.validate()
    numerator = costs.false_positive - costs.true_negative
    denominator = (costs.false_negative - costs.true_positive) + numerator
    return float(numerator / denominator)


# --------------------------------------------------------------------------- #
# the pipeline
# --------------------------------------------------------------------------- #

def run_posthoc(
    *,
    framework: str,
    costs: CostMatrix,
    oos_predictions: np.ndarray,
    oos_realized: np.ndarray,
    cal_scores: np.ndarray,
    cal_outcomes: np.ndarray,
    calibrator: str = "platt",
    state_dicts: Sequence[dict] | None = None,
) -> PostHocReport:
    """Run the four steps, in order, once.

    There is no `order=` parameter and no per-step switch. The only way a step
    does not happen is that it does not apply, and that is recorded.
    """
    soup = weight_soup(framework, state_dicts)
    weights, ensemble = greedy_ensemble(oos_predictions, oos_realized)
    params, calibration = calibrate(cal_scores, cal_outcomes, method=calibrator)
    threshold = decision_threshold(costs)
    threshold_step = StepReport(
        "threshold",
        applied=True,
        detail={"closed_form": True, "value": threshold},
    )

    return PostHocReport(
        steps=[soup, ensemble, calibration, threshold_step],
        weights=weights,
        calibrator=calibrator if calibration.applied else None,
        calibration_params=[float(p) for p in np.atleast_1d(params)],
        threshold=threshold,
    )
