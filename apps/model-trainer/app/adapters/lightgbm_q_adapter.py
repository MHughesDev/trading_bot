"""LightGBM quantile-regression adapter (I-1.5).

Trains one LightGBM model per quantile level using `objective=quantile` and
`alpha=level`. Outputs the full sorted quantile vector in σ-units. Repair is
applied after prediction (I-1.8). Distribution contract is validated before
the bundle is written (I-1.12).
"""

import pickle

import lightgbm as lgb
import numpy as np

from .. import engine
from .. import hpo as hpo_mod


def train(definition: dict, df, emit_progress) -> tuple[bytes, dict]:
    engine.seed_everything(definition)

    # HPO pass (I-1.9): finds best hyperparameters before the final fit.
    if hpo_mod.is_enabled(definition):
        folds = [definition.get("_wf_fold")] if definition.get("_wf_fold") else []
        definition, n_trials = hpo_mod.run_hpo(
            definition, df, folds, train, emit_progress
        )
    else:
        n_trials = 0

    p = engine.prepare(definition, df)
    levels = p.quantile_levels
    if not levels:
        levels = [0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95]
    hp = p.hp

    base_params = {
        "max_depth": int(hp.get("max_depth", -1)),
        "num_leaves": int(hp.get("num_leaves", 31)),
        "learning_rate": float(hp.get("learning_rate", 0.05)),
        "feature_fraction": float(hp.get("feature_fraction", 1.0)),
        "bagging_fraction": float(hp.get("bagging_fraction", 1.0)),
        "min_child_samples": int(hp.get("min_child_samples", 20)),
        "lambda_l1": float(hp.get("lambda_l1", float(hp.get("reg_alpha", 0.0)))),
        "lambda_l2": float(hp.get("lambda_l2", float(hp.get("reg_lambda", 0.0)))),
        "seed": p.seed,
        "verbose": -1,
        "metric": "quantile",
    }
    n_estimators = int(hp.get("n_estimators", 200))

    model_strings: list[str] = []
    for idx, alpha in enumerate(levels):
        params = {**base_params, "objective": "quantile", "alpha": alpha}
        dtrain = lgb.Dataset(p.X_tr, label=p.y_tr)
        valid_sets = [dtrain]
        valid_names = ["train"]
        if len(p.X_val) > 0:
            dval = lgb.Dataset(p.X_val, label=p.y_val, reference=dtrain)
            valid_sets.append(dval)
            valid_names.append("validation")

        pct_start = (idx / len(levels)) * 90.0
        pct_end = ((idx + 1) / len(levels)) * 90.0

        def _progress(env, _start=pct_start, _end=pct_end, _n=n_estimators):
            i = env.iteration
            if i % max(1, _n // 10) == 0:
                pct = _start + (i / max(_n, 1)) * (_end - _start)
                emit_progress("fitting", pct, {"quantile_idx": idx, "round": i})

        booster = lgb.train(
            params,
            dtrain,
            num_boost_round=n_estimators,
            valid_sets=valid_sets,
            valid_names=valid_names,
            callbacks=[_progress],
        )
        model_strings.append(booster.model_to_string())

    def predict_all(X) -> np.ndarray:
        if len(X) == 0:
            return np.zeros((0, len(levels)))
        preds = []
        for ms, alpha in zip(model_strings, levels):
            b = lgb.Booster(model_str=ms)
            preds.append(b.predict(X))
        return np.column_stack(preds)

    q_val = predict_all(p.X_val)
    q_te = predict_all(p.X_te)

    # Repair crossing (I-1.8)
    q_val_rep, _ = engine.repair_quantiles(q_val)
    q_te_rep, n_repairs = engine.repair_quantiles(q_te)

    # Validate contract before writing bundle (I-1.12)
    if len(q_te_rep) > 0:
        engine.validate_distribution(q_te_rep, levels, p.sigma)

    parts = [
        engine.quantile_metrics("val", p.y_val, q_val_rep, levels),
        engine.quantile_metrics("test", p.y_te, q_te_rep, levels),
    ]
    metrics = engine.assemble_metrics(
        "regression",
        parts,
        framework_version=lgb.__version__,
        n_train=len(p.X_tr),
        n_val=len(p.X_val),
        n_test=len(p.X_te),
        extra={
            "arch": "lightgbm_q",
            "n_quantiles": len(levels),
            "quantile_repairs": n_repairs,
            "hpo_trials": n_trials,
            "sigma": p.sigma,
        },
    )

    inner = pickle.dumps({"models": model_strings, "levels": levels, "sigma": p.sigma})
    bundle = engine.wrap_bundle(
        inner,
        framework="lightgbm_q",
        objective="quantile",
        feature_order=p.feature_order,
        scaler=p.scaler,
        output_kind="distribution",
        quantile_levels=levels,
        sigma_scaler=p.sigma,
    )
    emit_progress("fitting", 100.0, metrics)
    return bundle, metrics
