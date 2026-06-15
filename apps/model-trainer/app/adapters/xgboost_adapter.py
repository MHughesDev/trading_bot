import tempfile

import numpy as np
import xgboost as xgb

from .base import split_label, train_val_split


def _binarize(y):
    """Coerce y to a binary 0/1 label. If continuous (>2 unique), split at median."""
    nunique = y.nunique(dropna=True)
    if nunique > 2:
        y = (y > y.median()).astype(int)
    else:
        y = y.astype(int)
    return y


def train(definition: dict, df, emit_progress) -> tuple[bytes, dict]:
    hp = definition.get("hyperparameters", {}) or {}

    X, y = split_label(df)
    y = _binarize(y)

    X_tr, y_tr, X_val, y_val = train_val_split(X, y, frac=0.8)

    dtrain = xgb.DMatrix(X_tr, label=y_tr)
    dval = xgb.DMatrix(X_val, label=y_val)

    n_estimators = int(hp.get("n_estimators", 200))
    max_depth = int(hp.get("max_depth", 6))
    learning_rate = float(hp.get("learning_rate", 0.05))

    params = {
        "objective": "binary:logistic",
        "eval_metric": ["auc", "logloss"],
        "max_depth": max_depth,
        "eta": learning_rate,
    }

    evals_result: dict = {}

    def callback(env):
        i = env.iteration
        if i % 10 == 0:
            pct = (i / max(n_estimators, 1)) * 100.0
            metric = {"round": i}
            for name, vals in env.evaluation_result_list:
                metric[name] = float(vals)
            emit_progress("fitting", pct, metric)

    booster = xgb.train(
        params,
        dtrain,
        num_boost_round=n_estimators,
        evals=[(dtrain, "train"), (dval, "validation")],
        evals_result=evals_result,
        callbacks=[callback],
        verbose_eval=False,
    )

    # Single-class guard for AUC
    val_auc = None
    val_logloss = None
    try:
        val_metrics = evals_result.get("validation", {})
        if len(np.unique(y_val)) >= 2 and val_metrics.get("auc"):
            val_auc = float(val_metrics["auc"][-1])
        if val_metrics.get("logloss"):
            val_logloss = float(val_metrics["logloss"][-1])
    except Exception:
        pass

    feature_importance = {k: float(v) for k, v in booster.get_score(importance_type="gain").items()}

    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tmp:
        booster.save_model(tmp.name)
        tmp.flush()
        with open(tmp.name, "rb") as f:
            artifact_bytes = f.read()

    metrics = {
        "val_auc": val_auc,
        "val_logloss": val_logloss,
        "n_train": int(len(X_tr)),
        "n_val": int(len(X_val)),
        "feature_importance": feature_importance,
        "framework_version": xgb.__version__,
    }
    return artifact_bytes, metrics
