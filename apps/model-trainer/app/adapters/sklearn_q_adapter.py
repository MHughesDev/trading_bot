"""sklearn quantile-regression adapter (I-1.5).

Uses `GradientBoostingRegressor(loss="quantile")` — one model per quantile level.
"""

import pickle

import numpy as np
from sklearn.ensemble import GradientBoostingRegressor

from .. import engine
from .. import hpo as hpo_mod


def train(definition: dict, df, emit_progress) -> tuple[bytes, dict]:
    import sklearn

    engine.seed_everything(definition)

    if hpo_mod.is_enabled(definition):
        folds = [definition.get("_wf_fold")] if definition.get("_wf_fold") else []
        definition, n_trials = hpo_mod.run_hpo(
            definition, df, folds, train, emit_progress
        )
    else:
        n_trials = 0

    p = engine.prepare(definition, df)
    levels = p.quantile_levels or [0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95]
    hp = p.hp

    n_estimators = int(hp.get("n_estimators", 200))
    base_kwargs = {
        "n_estimators": n_estimators,
        "max_depth": int(hp.get("max_depth", 3)),
        "learning_rate": float(hp.get("learning_rate", 0.1)),
        "subsample": float(hp.get("subsample", 1.0)),
        "min_samples_leaf": int(hp.get("min_samples_leaf", 1)),
        "random_state": p.seed,
    }

    models: list = []
    for idx, alpha in enumerate(levels):
        m = GradientBoostingRegressor(loss="quantile", alpha=alpha, **base_kwargs)
        m.fit(p.X_tr, p.y_tr)
        models.append(m)
        pct = ((idx + 1) / len(levels)) * 90.0
        emit_progress("fitting", pct, {"quantile_idx": idx})

    def predict_all(X) -> np.ndarray:
        if len(X) == 0:
            return np.zeros((0, len(levels)))
        return np.column_stack([m.predict(X) for m in models])

    q_val = predict_all(p.X_val)
    q_te = predict_all(p.X_te)
    q_val_rep, _ = engine.repair_quantiles(q_val)
    q_te_rep, n_repairs = engine.repair_quantiles(q_te)

    if len(q_te_rep) > 0:
        engine.validate_distribution(q_te_rep, levels, p.sigma)

    parts = [
        engine.quantile_metrics("val", p.y_val, q_val_rep, levels),
        engine.quantile_metrics("test", p.y_te, q_te_rep, levels),
    ]
    metrics = engine.assemble_metrics(
        "regression",
        parts,
        framework_version=sklearn.__version__,
        n_train=len(p.X_tr),
        n_val=len(p.X_val),
        n_test=len(p.X_te),
        extra={
            "arch": "sklearn_q",
            "n_quantiles": len(levels),
            "quantile_repairs": n_repairs,
            "hpo_trials": n_trials,
            "sigma": p.sigma,
        },
    )

    inner = pickle.dumps({"models": models, "levels": levels, "sigma": p.sigma})
    bundle = engine.wrap_bundle(
        inner,
        framework="sklearn_q",
        objective="quantile",
        feature_order=p.feature_order,
        scaler=p.scaler,
        output_kind="distribution",
        quantile_levels=levels,
        sigma_scaler=p.sigma,
    )
    emit_progress("fitting", 100.0, metrics)
    return bundle, metrics
