import io

import joblib
import numpy as np

from ..schemas import Forecast
from . import base


def predict(
    artifact_bytes: bytes,
    instances: list,
    model_kind: str,
    horizon: str,
    header: dict | None = None,
) -> list[Forecast]:
    X, objective = base.build_matrix(instances, header)

    model = joblib.load(io.BytesIO(artifact_bytes))

    if objective == "regression":
        preds = np.atleast_1d(np.asarray(model.predict(X), dtype=float).ravel())
        return [base.to_forecast_return(float(v), horizon) for v in preds]

    if hasattr(model, "predict_proba"):
        proba = np.asarray(model.predict_proba(X))
        scores = proba[:, 1] if proba.ndim == 2 and proba.shape[1] >= 2 else proba.ravel()
    elif hasattr(model, "decision_function"):
        raw = np.asarray(model.decision_function(X), dtype=float).ravel()
        scores = 1.0 / (1.0 + np.exp(-raw))
    else:
        scores = np.asarray(model.predict(X), dtype=float).ravel()

    scores = np.atleast_1d(scores)
    return [base.to_forecast(float(s), horizon) for s in scores]
