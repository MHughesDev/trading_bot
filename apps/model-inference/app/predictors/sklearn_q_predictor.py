"""sklearn quantile predictor (I-1.11)."""

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
    import pickle

    payload = pickle.loads(artifact_bytes)
    models = payload["models"]
    levels: list[float] = payload["levels"]
    sigma: float = float(payload.get("sigma", 1.0))

    feature_order, scaler = base.resolve_features(instances, header)
    X = base.features_matrix(instances, feature_order=feature_order)
    X = base.apply_scaler(X, scaler)

    if len(X) == 0:
        return []

    q_sigma = np.column_stack([m.predict(X) for m in models])
    q_sigma = np.sort(q_sigma, axis=1)

    return [base.to_distribution_forecast(q_sigma[i], levels, sigma, horizon) for i in range(len(X))]
