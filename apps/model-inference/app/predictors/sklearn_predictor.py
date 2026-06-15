import io

import joblib
import numpy as np

from ..schemas import Forecast
from . import base


def predict(artifact_bytes: bytes, instances: list, model_kind: str, horizon: str) -> list[Forecast]:
    keys = set()
    for inst in instances:
        f = getattr(inst, "features", {}) or {}
        keys.update(f.keys())
    feature_order = sorted(keys)

    X = base.features_matrix(instances, feature_order=feature_order)

    model = joblib.load(io.BytesIO(artifact_bytes))

    if hasattr(model, "predict_proba"):
        proba = model.predict_proba(X)
        proba = np.asarray(proba)
        if proba.ndim == 2 and proba.shape[1] >= 2:
            scores = proba[:, 1]
        else:
            scores = proba.ravel()
    elif hasattr(model, "decision_function"):
        raw = np.asarray(model.decision_function(X), dtype=float).ravel()
        # Squash decision scores into (0, 1) via logistic for a probability-like value.
        scores = 1.0 / (1.0 + np.exp(-raw))
    else:
        scores = np.asarray(model.predict(X), dtype=float).ravel()

    scores = np.atleast_1d(scores)
    return [base.to_forecast(float(s), horizon) for s in scores]
