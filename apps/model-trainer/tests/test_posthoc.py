"""Tests for the post-hoc pipeline (SPEC 11.4, AT-35).

Run with: python -m pytest apps/model-trainer/tests -q
"""

from __future__ import annotations

import pathlib
import sys

import numpy as np
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))

from app import failures  # noqa: E402
from app import posthoc  # noqa: E402


def costs(**over) -> posthoc.CostMatrix:
    base = dict(true_positive=-1.0, false_positive=1.0, true_negative=0.0, false_negative=3.0)
    base.update(over)
    return posthoc.CostMatrix(**base)


# --------------------------------------------------------------------------- #
# AT-35 - the threshold is closed-form
# --------------------------------------------------------------------------- #

def test_the_threshold_is_the_cost_matrix_and_nothing_else():
    # t* = (C_fp - C_tn) / ((C_fn - C_tp) + (C_fp - C_tn))
    #    = (1 - 0) / ((3 - -1) + (1 - 0)) = 1/5
    assert posthoc.decision_threshold(costs()) == pytest.approx(0.2)

    # It depends only on the costs: the same matrix gives the same number
    # whatever data is around, because it never sees any.
    assert posthoc.decision_threshold(costs()) == posthoc.decision_threshold(costs())

    # Making a false negative more expensive lowers the bar for acting.
    cheap_miss = posthoc.decision_threshold(costs(false_negative=1.5))
    dear_miss = posthoc.decision_threshold(costs(false_negative=30.0))
    assert dear_miss < cheap_miss


def test_the_threshold_function_takes_no_data():
    """The signature is the guarantee: there is nothing to tune against."""
    import inspect

    params = list(inspect.signature(posthoc.decision_threshold).parameters)
    assert params == ["costs"], f"a threshold that can see outcomes can be tuned: {params}"


def test_a_degenerate_cost_matrix_is_refused():
    with pytest.raises(failures.TrainerFailure):
        posthoc.decision_threshold(costs(false_positive=-1.0))
    with pytest.raises(failures.TrainerFailure):
        posthoc.decision_threshold(costs(false_negative=-5.0))
    with pytest.raises(failures.TrainerFailure):
        posthoc.decision_threshold(costs(true_positive=float("nan")))


# --------------------------------------------------------------------------- #
# order
# --------------------------------------------------------------------------- #

def test_the_order_is_fixed_and_complete_even_when_a_step_does_not_apply():
    rng = np.random.default_rng(7)
    truth = rng.normal(size=400)
    preds = np.vstack([truth + rng.normal(scale=0.5, size=400) for _ in range(4)])
    cal_scores = rng.uniform(0.05, 0.95, size=200)
    cal_outcomes = (rng.uniform(size=200) < cal_scores).astype(float)

    report = posthoc.run_posthoc(
        framework="lightgbm",
        costs=costs(),
        oos_predictions=preds,
        oos_realized=truth,
        cal_scores=cal_scores,
        cal_outcomes=cal_outcomes,
    )
    assert report.order() == posthoc.STEPS
    soup = report.steps[0]
    assert not soup.applied and soup.skipped == "not_applicable", (
        "a GBDT still has four steps; the soup is recorded as skipped, not dropped"
    )
    assert report.threshold == pytest.approx(0.2)


def test_run_posthoc_has_no_ordering_parameter():
    import inspect

    params = set(inspect.signature(posthoc.run_posthoc).parameters)
    for forbidden in ("order", "steps", "skip", "reorder", "enable"):
        assert forbidden not in params, f"{forbidden!r} would make the order a choice"


# --------------------------------------------------------------------------- #
# greedy ensemble
# --------------------------------------------------------------------------- #

def test_greedy_selection_puts_its_weight_on_the_model_that_earns_it():
    rng = np.random.default_rng(11)
    truth = rng.normal(size=500)
    good = truth + rng.normal(scale=0.1, size=500)
    bad = truth + rng.normal(scale=3.0, size=500)
    noise = rng.normal(size=500)

    weights, step = posthoc.greedy_ensemble(np.vstack([good, bad, noise]), truth)
    assert step.applied
    assert weights.sum() == pytest.approx(1.0)
    assert weights[0] > 0.5, f"the accurate member should dominate: {weights}"
    assert weights[0] > weights[2]


def test_selection_with_replacement_can_weight_unequally():
    """Without replacement the best a member can get is 1/k of the weight."""
    rng = np.random.default_rng(3)
    truth = rng.normal(size=300)
    good = truth + rng.normal(scale=0.05, size=300)
    weak = truth * 0.2 + rng.normal(scale=1.0, size=300)
    weights, _ = posthoc.greedy_ensemble(np.vstack([good, weak]), truth, rounds=20)
    assert weights[0] not in (0.0, 0.5, 1.0) or weights[0] > 0.5


def test_a_single_member_is_not_an_ensemble():
    truth = np.arange(50, dtype=float)
    weights, step = posthoc.greedy_ensemble(truth[None, :], truth)
    assert weights.tolist() == [1.0]
    assert not step.applied and step.skipped == "single_member"


def test_ragged_predictions_are_refused():
    with pytest.raises(failures.DataError):
        posthoc.greedy_ensemble(np.zeros((3, 10)), np.zeros(11))


# --------------------------------------------------------------------------- #
# calibration
# --------------------------------------------------------------------------- #

def test_calibration_corrects_a_systematically_overconfident_model():
    rng = np.random.default_rng(5)
    true_p = rng.uniform(0.1, 0.9, size=2000)
    outcomes = (rng.uniform(size=2000) < true_p).astype(float)
    # An overconfident model: probabilities pushed away from 1/2.
    raw = np.clip(true_p + 0.35 * np.sign(true_p - 0.5), 0.01, 0.99)

    params, step = posthoc.calibrate(raw, outcomes, method="platt")
    assert step.applied
    fixed = posthoc.apply_calibration(raw, "platt", params)
    before = abs(raw.mean() - outcomes.mean())
    after = abs(fixed.mean() - outcomes.mean())
    assert after < before, f"calibration must move toward the base rate: {before} -> {after}"


def test_a_binning_calibrator_cannot_be_asked_for():
    rng = np.random.default_rng(2)
    scores = rng.uniform(0.05, 0.95, size=100)
    outcomes = (rng.uniform(size=100) < scores).astype(float)
    for banned in ("isotonic", "histogram", "binning"):
        with pytest.raises(failures.TrainerFailure):
            posthoc.calibrate(scores, outcomes, method=banned)


def test_calibration_refuses_rather_than_pretends_on_a_thin_set():
    rng = np.random.default_rng(1)
    scores = rng.uniform(0.05, 0.95, size=12)
    outcomes = (rng.uniform(size=12) < scores).astype(float)
    _, step = posthoc.calibrate(scores, outcomes)
    assert not step.applied and step.skipped == "calibration_set_too_small"

    scores = rng.uniform(0.05, 0.95, size=100)
    _, step = posthoc.calibrate(scores, np.ones(100))
    assert not step.applied and step.skipped == "one_class_only"


def test_every_offered_calibrator_is_monotone():
    rng = np.random.default_rng(9)
    true_p = rng.uniform(0.05, 0.95, size=1500)
    outcomes = (rng.uniform(size=1500) < true_p).astype(float)
    grid = np.linspace(0.02, 0.98, 50)
    for method in posthoc.CALIBRATORS:
        params, step = posthoc.calibrate(true_p, outcomes, method=method)
        assert step.applied, method
        mapped = posthoc.apply_calibration(grid, method, params)
        assert np.all(np.diff(mapped) >= -1e-9), f"{method} is not monotone: {mapped}"
