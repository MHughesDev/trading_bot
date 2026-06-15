import os
import tempfile

import numpy as np
import lightgbm as lgb

from ..schemas import Forecast
from . import base


def _load_booster(artifact_bytes: bytes):
    # Prefer loading from an in-memory model string (text model).
    try:
        model_str = artifact_bytes.decode("utf-8")
        if "tree" in model_str or "version" in model_str:
            return lgb.Booster(model_str=model_str)
    except Exception:
        pass

    # Fall back to writing to a tempfile and loading via model_file.
    path = None
    try:
        with tempfile.NamedTemporaryFile(suffix=".txt", delete=False) as tmp:
            tmp.write(artifact_bytes)
            tmp.flush()
            path = tmp.name
        return lgb.Booster(model_file=path)
    finally:
        if path and os.path.exists(path):
            try:
                os.unlink(path)
            except OSError:
                pass


def predict(artifact_bytes: bytes, instances: list, model_kind: str, horizon: str) -> list[Forecast]:
    keys = set()
    for inst in instances:
        f = getattr(inst, "features", {}) or {}
        keys.update(f.keys())
    feature_order = sorted(keys)

    X = base.features_matrix(instances, feature_order=feature_order)

    booster = _load_booster(artifact_bytes)
    probs = booster.predict(X)
    probs = np.atleast_1d(np.asarray(probs, dtype=float))

    return [base.to_forecast(float(p), horizon) for p in probs]
