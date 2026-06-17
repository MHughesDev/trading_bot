"""XGBoost quantile-regression adapter (I-1.5).

Uses `objective=reg:quantileerror` (XGBoost ≥ 2.0) with `quantile_alpha` list
for multi-quantile output in a single model. Falls back to per-level training
when the multi-quantile API is unavailable.
"""

import tempfile

import numpy as np
import xgboost as xgb

from .. import engine
from .. import hpo as hpo_mod


def _xgb_version_ge(major: int, minor: int = 0) -> bool:
    try:
        parts = xgb.__version__.split(".")
        return (int(parts[0]), int(parts[1])) >= (major, minor)
    except Exception:  # noqa: BLE001
        return False


def _train_multi_quantile(p, hp, levels, n_estimators, emit_progress):
    """Single model, multi-quantile output (XGBoost ≥ 2.0)."""
    params = {
        "objective": "reg:quantileerror",
        "quantile_alpha": levels,
        "max_depth": int(hp.get("max_depth", 6)),
        "eta": float(hp.get("eta", hp.get("learning_rate", 0.05))),
        "subsample": float(hp.get("subsample", 1.0)),
        "colsample_bytree": float(hp.get("colsample_bytree", 1.0)),
        "min_child_weight": float(hp.get("min_child_weight", 1.0)),
        "reg_alpha": float(hp.get("reg_alpha", 0.0)),
        "reg_lambda": float(hp.get("reg_lambda", 1.0)),
        "seed": p.seed,
    }
    dtrain = xgb.DMatrix(p.X_tr, label=p.y_tr)
    evals = [(dtrain, "train")]
    if len(p.X_val) > 0:
        evals.append((xgb.DMatrix(p.X_val, label=p.y_val), "validation"))

    class _Cb(xgb.callback.TrainingCallback):
        def after_iteration(self, model, epoch, evals_log):
            if epoch % max(1, n_estimators // 10) == 0:
                pct = (epoch / max(n_estimators, 1)) * 90.0
                emit_progress("fitting", pct, {"round": epoch})
            return False

    booster = xgb.train(
        params, dtrain,
        num_boost_round=n_estimators,
        evals=evals,
        verbose_eval=False,
        callbacks=[_Cb()],
    )
    return booster


def _predict_multi(booster, X) -> np.ndarray:
    raw = booster.predict(xgb.DMatrix(X))
    return raw.reshape(len(X), -1) if raw.ndim == 1 else raw


def train(definition: dict, df, emit_progress) -> tuple[bytes, dict]:
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

    if _xgb_version_ge(2, 0):
        booster = _train_multi_quantile(p, hp, levels, n_estimators, emit_progress)

        def predict_all(X) -> np.ndarray:
            if len(X) == 0:
                return np.zeros((0, len(levels)))
            return _predict_multi(booster, X)

        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tmp:
            booster.save_model(tmp.name)
            tmp.flush()
            with open(tmp.name, "rb") as f:
                inner = f.read()
        framework_tag = "xgboost_q"
    else:
        # Fallback: N separate single-quantile models stored as pickle.
        import pickle
        params_base = {
            "max_depth": int(hp.get("max_depth", 6)),
            "eta": float(hp.get("eta", hp.get("learning_rate", 0.05))),
            "subsample": float(hp.get("subsample", 1.0)),
            "colsample_bytree": float(hp.get("colsample_bytree", 1.0)),
            "seed": p.seed,
        }
        dtrain = xgb.DMatrix(p.X_tr, label=p.y_tr)
        boosters_raw = []
        for idx, alpha in enumerate(levels):
            params = {**params_base, "objective": "reg:quantileerror", "quantile_alpha": [alpha]}
            b = xgb.train(params, dtrain, num_boost_round=n_estimators, verbose_eval=False)
            with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tmp:
                b.save_model(tmp.name)
                tmp.flush()
                with open(tmp.name, "rb") as f:
                    boosters_raw.append(f.read())
            pct = ((idx + 1) / len(levels)) * 90.0
            emit_progress("fitting", pct, {"quantile_idx": idx})

        inner = pickle.dumps({"models": boosters_raw, "levels": levels, "sigma": p.sigma, "multi": False})
        framework_tag = "xgboost_q_compat"

        def predict_all(X) -> np.ndarray:
            if len(X) == 0:
                return np.zeros((0, len(levels)))
            preds = []
            for raw in boosters_raw:
                with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tmp2:
                    tmp2.write(raw)
                    tmp2.flush()
                    b2 = xgb.Booster()
                    b2.load_model(tmp2.name)
                p2 = b2.predict(xgb.DMatrix(X))
                preds.append(p2.ravel())
            return np.column_stack(preds)

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
        framework_version=xgb.__version__,
        n_train=len(p.X_tr),
        n_val=len(p.X_val),
        n_test=len(p.X_te),
        extra={
            "arch": "xgboost_q",
            "n_quantiles": len(levels),
            "quantile_repairs": n_repairs,
            "hpo_trials": n_trials,
            "sigma": p.sigma,
        },
    )

    if framework_tag == "xgboost_q":
        bundle = engine.wrap_bundle(
            inner,
            framework="xgboost_q",
            objective="quantile",
            feature_order=p.feature_order,
            scaler=p.scaler,
            output_kind="distribution",
            quantile_levels=levels,
            sigma_scaler=p.sigma,
        )
    else:
        bundle = engine.wrap_bundle(
            inner,
            framework="xgboost_q_compat",
            objective="quantile",
            feature_order=p.feature_order,
            scaler=p.scaler,
            output_kind="distribution",
            quantile_levels=levels,
            sigma_scaler=p.sigma,
        )
    emit_progress("fitting", 100.0, metrics)
    return bundle, metrics
