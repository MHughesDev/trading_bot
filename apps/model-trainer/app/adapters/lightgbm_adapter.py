import numpy as np
import lightgbm as lgb

from .. import engine


def train(definition: dict, df, emit_progress) -> tuple[bytes, dict]:
    engine.seed_everything(definition)
    p = engine.prepare(definition, df)
    hp = p.hp

    n_estimators = int(hp.get("n_estimators", 200))
    params = {
        "max_depth": int(hp.get("max_depth", -1)),
        "learning_rate": float(hp.get("learning_rate", 0.05)),
        "num_leaves": int(hp.get("num_leaves", 31)),
        "feature_fraction": float(hp.get("feature_fraction", 1.0)),
        "bagging_fraction": float(hp.get("bagging_fraction", 1.0)),
        "min_child_samples": int(hp.get("min_child_samples", 20)),
        "lambda_l1": float(hp.get("reg_alpha", 0.0)),
        "lambda_l2": float(hp.get("reg_lambda", 0.0)),
        "seed": p.seed,
        "verbose": -1,
    }
    if p.objective == "classification":
        params.update(objective="binary", metric=["auc", "binary_logloss"])
    else:
        params.update(objective="regression", metric=["l2", "l1"])

    dtrain = lgb.Dataset(p.X_tr, label=p.y_tr)
    valid_sets = [dtrain]
    valid_names = ["train"]
    has_val = len(p.X_val) > 0
    if has_val:
        dval = lgb.Dataset(p.X_val, label=p.y_val, reference=dtrain)
        valid_sets.append(dval)
        valid_names.append("validation")

    def progress_callback(env):
        i = env.iteration
        if i % 10 == 0:
            pct = (i / max(n_estimators, 1)) * 100.0
            metric = {"round": i}
            for item in env.evaluation_result_list:
                metric[f"{item[0]}_{item[1]}"] = float(item[2])
            emit_progress("fitting", pct, metric)

    callbacks = [progress_callback]
    if has_val:
        early = int(hp.get("early_stopping_rounds", 30))
        if early > 0:
            callbacks.append(lgb.early_stopping(stopping_rounds=early, verbose=False))

    booster = lgb.train(
        params,
        dtrain,
        num_boost_round=n_estimators,
        valid_sets=valid_sets,
        valid_names=valid_names,
        callbacks=callbacks,
    )

    def predict(X):
        if len(X) == 0:
            return np.array([])
        return booster.predict(X)

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

    metrics = engine.assemble_metrics(
        p.objective,
        parts,
        framework_version=lgb.__version__,
        n_train=len(p.X_tr),
        n_val=len(p.X_val),
        n_test=len(p.X_te),
        extra={"best_iteration": booster.best_iteration},
    )

    inner = booster.model_to_string().encode()
    bundle = engine.wrap_bundle(
        inner,
        framework="lightgbm",
        objective=p.objective,
        feature_order=p.feature_order,
        scaler=p.scaler,
    )
    return bundle, metrics
