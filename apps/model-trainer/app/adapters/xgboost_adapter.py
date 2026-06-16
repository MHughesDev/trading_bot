import tempfile

import numpy as np
import xgboost as xgb

from .. import engine


def train(definition: dict, df, emit_progress) -> tuple[bytes, dict]:
    engine.seed_everything(definition)
    p = engine.prepare(definition, df)
    hp = p.hp

    n_estimators = int(hp.get("n_estimators", 200))
    params = {
        "max_depth": int(hp.get("max_depth", 6)),
        "eta": float(hp.get("learning_rate", 0.05)),
        "subsample": float(hp.get("subsample", 1.0)),
        "colsample_bytree": float(hp.get("colsample_bytree", 1.0)),
        "min_child_weight": float(hp.get("min_child_weight", 1.0)),
        "gamma": float(hp.get("gamma", 0.0)),
        "reg_alpha": float(hp.get("reg_alpha", 0.0)),
        "reg_lambda": float(hp.get("reg_lambda", 1.0)),
        "seed": p.seed,
    }
    if p.objective == "classification":
        params.update(objective="binary:logistic", eval_metric=["auc", "logloss"])
    else:
        params.update(objective="reg:squarederror", eval_metric="rmse")

    dtrain = xgb.DMatrix(p.X_tr, label=p.y_tr)
    evals = [(dtrain, "train")]
    dval = None
    if len(p.X_val):
        dval = xgb.DMatrix(p.X_val, label=p.y_val)
        evals.append((dval, "validation"))

    early = int(hp.get("early_stopping_rounds", 30)) if dval is not None else 0

    class ProgressCallback(xgb.callback.TrainingCallback):
        def after_iteration(self, model, epoch, evals_log):
            if epoch % 10 == 0:
                pct = (epoch / max(n_estimators, 1)) * 100.0
                metric = {"round": epoch}
                for dataset, metrics in evals_log.items():
                    for name, vals in metrics.items():
                        metric[f"{dataset}_{name}"] = float(vals[-1])
                emit_progress("fitting", pct, metric)
            return False

    evals_result: dict = {}
    booster = xgb.train(
        params,
        dtrain,
        num_boost_round=n_estimators,
        evals=evals,
        evals_result=evals_result,
        early_stopping_rounds=early or None,
        callbacks=[ProgressCallback()],
        verbose_eval=False,
    )

    def predict(X):
        if len(X) == 0:
            return np.array([])
        return booster.predict(xgb.DMatrix(X))

    if p.objective == "classification":
        parts = [
            engine.classification_metrics("val", p.y_val, predict(p.X_val)),
            engine.classification_metrics("test", p.y_te, predict(p.X_te)),
        ]
    else:
        parts = [
            engine.regression_metrics("val", p.y_val, predict(p.X_val)),
            engine.regression_metrics("test", p.y_te, predict(p.X_te)),
        ]

    feature_importance = {
        k: float(v) for k, v in booster.get_score(importance_type="gain").items()
    }
    metrics = engine.assemble_metrics(
        p.objective,
        parts,
        framework_version=xgb.__version__,
        n_train=len(p.X_tr),
        n_val=len(p.X_val),
        n_test=len(p.X_te),
        extra={
            "feature_importance": feature_importance,
            "best_iteration": getattr(booster, "best_iteration", None),
        },
    )

    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tmp:
        booster.save_model(tmp.name)
        tmp.flush()
        with open(tmp.name, "rb") as f:
            inner = f.read()

    bundle = engine.wrap_bundle(
        inner,
        framework="xgboost",
        objective=p.objective,
        feature_order=p.feature_order,
        scaler=p.scaler,
    )
    return bundle, metrics
