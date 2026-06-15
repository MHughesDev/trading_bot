import numpy as np
import lightgbm as lgb

from .base import split_label, train_val_split


def _binarize(y):
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

    n_estimators = int(hp.get("n_estimators", 200))
    max_depth = int(hp.get("max_depth", -1))
    learning_rate = float(hp.get("learning_rate", 0.05))

    dtrain = lgb.Dataset(X_tr, label=y_tr)
    dval = lgb.Dataset(X_val, label=y_val, reference=dtrain)

    params = {
        "objective": "binary",
        "metric": ["auc", "binary_logloss"],
        "max_depth": max_depth,
        "learning_rate": learning_rate,
        "verbose": -1,
    }

    evals_result: dict = {}

    def progress_callback(env):
        i = env.iteration
        if i % 10 == 0:
            pct = (i / max(n_estimators, 1)) * 100.0
            metric = {"round": i}
            for item in env.evaluation_result_list:
                # item: (data_name, eval_name, result, is_higher_better)
                metric[f"{item[0]}_{item[1]}"] = float(item[2])
            emit_progress("fitting", pct, metric)

    booster = lgb.train(
        params,
        dtrain,
        num_boost_round=n_estimators,
        valid_sets=[dtrain, dval],
        valid_names=["train", "validation"],
        callbacks=[
            lgb.record_evaluation(evals_result),
            progress_callback,
        ],
    )

    val_auc = None
    val_logloss = None
    try:
        val_metrics = evals_result.get("validation", {})
        if len(np.unique(y_val)) >= 2 and val_metrics.get("auc"):
            val_auc = float(val_metrics["auc"][-1])
        if val_metrics.get("binary_logloss"):
            val_logloss = float(val_metrics["binary_logloss"][-1])
    except Exception:
        pass

    artifact_bytes = booster.model_to_string().encode()

    metrics = {
        "val_auc": val_auc,
        "val_logloss": val_logloss,
        "n_train": int(len(X_tr)),
        "n_val": int(len(X_val)),
        "framework_version": lgb.__version__,
    }
    return artifact_bytes, metrics
