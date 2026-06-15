import os
import tempfile

import numpy as np
import xgboost as xgb

from ..schemas import Forecast
from . import base


def predict(artifact_bytes: bytes, instances: list, model_kind: str, horizon: str) -> list[Forecast]:
    # Determine feature order deterministically (sorted union of keys).
    keys = set()
    for inst in instances:
        f = getattr(inst, "features", {}) or {}
        keys.update(f.keys())
    feature_order = sorted(keys)

    X = base.features_matrix(instances, feature_order=feature_order)

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

    dmat = xgb.DMatrix(X, feature_names=feature_order if feature_order else None)
    probs = booster.predict(dmat)
    probs = np.atleast_1d(np.asarray(probs, dtype=float))

    return [base.to_forecast(float(p), horizon) for p in probs]
