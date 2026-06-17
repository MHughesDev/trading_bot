"""XGBoost quantile predictor (I-1.11)."""

import tempfile

import numpy as np
import xgboost as xgb

from ..schemas import Forecast
from . import base


def predict(
    artifact_bytes: bytes,
    instances: list,
    model_kind: str,
    horizon: str,
    header: dict | None = None,
) -> list[Forecast]:
    levels: list[float] = (header or {}).get("quantile_levels") or []
    sigma: float = float((header or {}).get("sigma_scaler") or 1.0)

    feature_order, scaler = base.resolve_features(instances, header)
    X = base.features_matrix(instances, feature_order=feature_order)
    X = base.apply_scaler(X, scaler)

    if len(X) == 0:
        return []

    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tmp:
        tmp.write(artifact_bytes)
        tmp.flush()
        booster = xgb.Booster()
        booster.load_model(tmp.name)

    raw = booster.predict(xgb.DMatrix(X))
    # Multi-quantile output: shape (n_instances, n_quantiles)
    if raw.ndim == 1:
        raw = raw.reshape(len(X), -1)
    q_sigma = np.sort(raw, axis=1)

    if q_sigma.shape[1] != len(levels):
        # Shape mismatch — emit point forecast as fallback.
        return [base.to_forecast_return(float(q_sigma[i].mean()), horizon) for i in range(len(X))]

    return [base.to_distribution_forecast(q_sigma[i], levels, sigma, horizon) for i in range(len(X))]
