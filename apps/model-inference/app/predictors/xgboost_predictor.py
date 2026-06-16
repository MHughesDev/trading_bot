import os
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
    # Columns follow the training feature order (from the bundle) and are scaled
    # with the persisted scaler. The model was trained on positional numpy
    # columns, so we predict positionally too — no feature_names.
    X, objective = base.build_matrix(instances, header)

    path = None
    try:
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tmp:
            tmp.write(artifact_bytes)
            tmp.flush()
            path = tmp.name
        booster = xgb.Booster()
        booster.load_model(path)
    finally:
        if path and os.path.exists(path):
            try:
                os.unlink(path)
            except OSError:
                pass

    raw = np.atleast_1d(np.asarray(booster.predict(xgb.DMatrix(X)), dtype=float))

    if objective == "regression":
        return [base.to_forecast_return(float(v), horizon) for v in raw]
    return [base.to_forecast(float(p), horizon) for p in raw]
