"""In-fold HPO with Optuna (I-1.9).

Runs Optuna hyperparameter optimisation INSIDE the walk-forward folds, optimising
CRPS (mean pinball loss) on the test role. The calibration fold is never used for
HPO scoring. The trial count is persisted in run metrics for Phase 2 deflation.

Usage (from a quantile adapter):

    from .. import hpo as hpo_mod
    if hpo_mod.is_enabled(definition):
        best_def, n_trials = hpo_mod.run_hpo(definition, df, folds, train_fn, emit_progress)
    else:
        best_def, n_trials = definition, 0
"""

from __future__ import annotations

import logging
from typing import Callable

import numpy as np

from . import engine

log = logging.getLogger(__name__)

# Per-framework hyperparameter search spaces.
# Format: {param_name: (kind, low, high)} where kind ∈ {"int", "float", "float_log"}
_SEARCH_SPACES: dict[str, dict[str, tuple]] = {
    "lightgbm": {
        "num_leaves": ("int", 15, 127),
        "learning_rate": ("float_log", 0.005, 0.3),
        "n_estimators": ("int", 50, 500),
        "feature_fraction": ("float", 0.4, 1.0),
        "bagging_fraction": ("float", 0.5, 1.0),
        "min_child_samples": ("int", 5, 100),
        "lambda_l1": ("float_log", 1e-8, 10.0),
        "lambda_l2": ("float_log", 1e-8, 10.0),
    },
    "lightgbm_q": {
        "num_leaves": ("int", 15, 127),
        "learning_rate": ("float_log", 0.005, 0.3),
        "n_estimators": ("int", 50, 500),
        "feature_fraction": ("float", 0.4, 1.0),
        "min_child_samples": ("int", 5, 100),
    },
    "xgboost": {
        "max_depth": ("int", 3, 10),
        "eta": ("float_log", 0.005, 0.3),
        "n_estimators": ("int", 50, 500),
        "subsample": ("float", 0.5, 1.0),
        "colsample_bytree": ("float", 0.5, 1.0),
        "min_child_weight": ("float", 1.0, 10.0),
        "reg_alpha": ("float_log", 1e-8, 10.0),
        "reg_lambda": ("float_log", 1e-8, 10.0),
    },
    "xgboost_q": {
        "max_depth": ("int", 3, 10),
        "eta": ("float_log", 0.005, 0.3),
        "n_estimators": ("int", 50, 500),
        "subsample": ("float", 0.5, 1.0),
        "colsample_bytree": ("float", 0.5, 1.0),
    },
    "sklearn": {
        "max_depth": ("int", 2, 8),
        "learning_rate": ("float_log", 0.005, 0.3),
        "n_estimators": ("int", 50, 300),
        "subsample": ("float", 0.5, 1.0),
        "min_samples_leaf": ("int", 1, 50),
    },
    "sklearn_q": {
        "max_depth": ("int", 2, 8),
        "learning_rate": ("float_log", 0.005, 0.3),
        "n_estimators": ("int", 50, 300),
        "subsample": ("float", 0.5, 1.0),
    },
}


def is_enabled(definition: dict) -> bool:
    """Return True when the definition has HPO enabled."""
    hpo_cfg = definition.get("hpo") or {}
    return bool(hpo_cfg.get("enabled", False))


def _suggest_hyperparams(trial, framework: str) -> dict:
    """Sample hyperparameters from the framework search space."""
    space = _SEARCH_SPACES.get(framework, _SEARCH_SPACES.get("xgboost", {}))
    hp: dict = {}
    for name, spec in space.items():
        kind, low, high = spec
        if kind == "int":
            hp[name] = trial.suggest_int(name, int(low), int(high))
        elif kind == "float":
            hp[name] = trial.suggest_float(name, float(low), float(high))
        elif kind == "float_log":
            hp[name] = trial.suggest_float(name, float(low), float(high), log=True)
    return hp


def run_hpo(
    definition: dict,
    df,
    folds: list[dict] | None,
    train_fn: Callable,
    emit_progress,
) -> tuple[dict, int]:
    """Run in-fold HPO and return (best_definition, trial_count).

    If Optuna is not installed or HPO is disabled, returns the original
    definition unchanged and trial_count = 0.

    The test role of the provided folds is the ONLY split used for scoring;
    the calibration role is never touched (I-1.9 requirement).
    """
    hpo_cfg = definition.get("hpo") or {}
    if not hpo_cfg.get("enabled", False):
        return definition, 0

    try:
        import optuna  # type: ignore
    except ImportError:
        log.warning("optuna not installed — HPO skipped")
        return definition, 0

    optuna.logging.set_verbosity(optuna.logging.WARNING)

    max_trials = int(hpo_cfg.get("max_trials", 20))
    seed = engine.resolve_seed(definition)
    framework = str(definition.get("framework", "xgboost")).lower()

    def objective(trial):
        hp = _suggest_hyperparams(trial, framework)
        trial_def = {**definition, "hyperparameters": {**(definition.get("hyperparameters") or {}), **hp}}
        scores: list[float] = []
        for fold_dict in (folds or [{}]):
            fold_def = {**trial_def, "_wf_fold": fold_dict} if fold_dict else trial_def
            try:
                _artifact_bytes, metrics = train_fn(fold_def, df, lambda *a: None)
                # Prefer CRPS for quantile models, fall back to MAE.
                score = (
                    metrics.get("test_crps")
                    or metrics.get("crps")
                    or metrics.get("test_mae")
                    or metrics.get("mae")
                    or float("inf")
                )
                scores.append(float(score))
            except Exception as e:  # noqa: BLE001
                log.debug("HPO trial failed: %s", e)
                return float("inf")
        return float(np.mean(scores)) if scores else float("inf")

    sampler = optuna.samplers.TPESampler(seed=seed)
    study = optuna.create_study(direction="minimize", sampler=sampler)

    n_trials_done = 0
    for i in range(max_trials):
        pct = (i / max(max_trials, 1)) * 100.0
        emit_progress("hpo", pct, {"hpo_trial": i + 1})
        try:
            study.optimize(objective, n_trials=1, show_progress_bar=False)
            n_trials_done += 1
        except Exception as e:  # noqa: BLE001
            log.warning("HPO trial %d exception: %s", i, e)

    best_hp = study.best_params if n_trials_done > 0 else {}
    best_def = {
        **definition,
        "hyperparameters": {**(definition.get("hyperparameters") or {}), **best_hp},
    }
    log.info("HPO done: %d trials, best value=%.6f", n_trials_done, study.best_value)
    return best_def, n_trials_done
