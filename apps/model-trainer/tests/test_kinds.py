"""Generic model kinds train with the right objective (Set U U-5.7)."""
from __future__ import annotations

import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from app.engine import resolve_objective  # noqa: E402


def test_a_regressor_trains_as_a_regression():
    # Before U-5.7 a regressor fell through to classification and was trained
    # on a thresholded label — its "value" was a probability.
    assert resolve_objective({"model_kind": "regressor"}) == "regression"


def test_a_classifier_trains_as_a_classification():
    assert resolve_objective({"model_kind": "classifier"}) == "classification"


def test_an_explicit_objective_still_wins():
    assert resolve_objective({"model_kind": "regressor", "hyperparameters": {"objective": "classification"}}) == "classification"
