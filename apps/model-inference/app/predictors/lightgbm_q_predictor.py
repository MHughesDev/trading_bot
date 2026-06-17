"""LightGBM quantile predictor (I-1.11).

Loads the N per-level LightGBM models from the bundle's inner pickle,
predicts all quantile levels, repairs crossing, validates, and emits
a distributional Forecast.
"""

import numpy as np
import lightgbm as lgb

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
    model_strings: list[str] = payload["models"]
    levels: list[float] = payload["levels"]
    sigma: float = float(payload.get("sigma", 1.0))

    feature_order, scaler = base.resolve_features(instances, header)
    X = base.features_matrix(instances, feature_order=feature_order)
    X = base.apply_scaler(X, scaler)

    if len(X) == 0:
        return []

    preds = []
    for ms in model_strings:
        b = lgb.Booster(model_str=ms)
        preds.append(b.predict(X))
    q_sigma = np.column_stack(preds)  # (n_instances, n_quantiles)

    # Repair crossing (I-1.8)
    q_sigma_sorted = np.sort(q_sigma, axis=1)

    results = []
    for row in q_sigma_sorted:
        fc = base.to_distribution_forecast(row, levels, sigma, horizon)
        results.append(fc)
    return results
