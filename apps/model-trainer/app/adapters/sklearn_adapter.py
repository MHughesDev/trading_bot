import io

import joblib
import numpy as np
import sklearn
from sklearn.ensemble import (
    GradientBoostingClassifier,
    GradientBoostingRegressor,
    RandomForestClassifier,
    RandomForestRegressor,
)
from sklearn.linear_model import LogisticRegression, Ridge

from .. import engine

# Estimator name -> (classifier, regressor) so the same UI choice maps to the
# right family once the objective is known.
_ESTIMATORS = {
    "RandomForestClassifier": (RandomForestClassifier, RandomForestRegressor),
    "GradientBoostingClassifier": (GradientBoostingClassifier, GradientBoostingRegressor),
    "LogisticRegression": (LogisticRegression, Ridge),
}

# Kwargs that aren't estimator constructor params.
_RESERVED = {"estimator", "objective", "direction_threshold", "seed", "early_stopping_rounds"}


def train(definition: dict, df, emit_progress) -> tuple[bytes, dict]:
    engine.seed_everything(definition)
    p = engine.prepare(definition, df)
    hp = p.hp

    estimator_name = hp.get("estimator", "RandomForestClassifier")
    classifier_cls, regressor_cls = _ESTIMATORS.get(
        estimator_name, (RandomForestClassifier, RandomForestRegressor)
    )
    estimator_cls = classifier_cls if p.objective == "classification" else regressor_cls

    kwargs = {k: v for k, v in hp.items() if k not in _RESERVED}
    # Seed estimators that accept random_state for reproducibility.
    if "random_state" not in kwargs:
        kwargs["random_state"] = p.seed
    try:
        model = estimator_cls(**kwargs)
    except TypeError:
        try:
            model = estimator_cls(random_state=p.seed)
        except TypeError:
            model = estimator_cls()

    emit_progress("fitting", 40.0, {"estimator": estimator_cls.__name__})
    model.fit(p.X_tr, p.y_tr)
    emit_progress("evaluating", 80.0, {"estimator": estimator_cls.__name__})

    def predict_scores(X):
        if len(X) == 0:
            return np.array([])
        if p.objective == "classification":
            if hasattr(model, "predict_proba"):
                proba = np.asarray(model.predict_proba(X))
                return proba[:, 1] if proba.ndim == 2 and proba.shape[1] >= 2 else proba.ravel()
            if hasattr(model, "decision_function"):
                raw = np.asarray(model.decision_function(X), dtype=float).ravel()
                return 1.0 / (1.0 + np.exp(-raw))
        return np.asarray(model.predict(X), dtype=float).ravel()

    if p.objective == "classification":
        parts = [
            engine.classification_metrics("val", p.y_val, predict_scores(p.X_val)),
            engine.classification_metrics("test", p.y_te, predict_scores(p.X_te)),
        ]
    else:
        parts = [
            engine.regression_metrics("val", p.y_val, predict_scores(p.X_val)),
            engine.regression_metrics("test", p.y_te, predict_scores(p.X_te)),
        ]

    metrics = engine.assemble_metrics(
        p.objective,
        parts,
        framework_version=sklearn.__version__,
        n_train=len(p.X_tr),
        n_val=len(p.X_val),
        n_test=len(p.X_te),
        extra={"estimator": estimator_cls.__name__},
    )

    buf = io.BytesIO()
    joblib.dump(model, buf)
    bundle = engine.wrap_bundle(
        buf.getvalue(),
        framework="sklearn",
        objective=p.objective,
        feature_order=p.feature_order,
        scaler=p.scaler,
    )
    return bundle, metrics
