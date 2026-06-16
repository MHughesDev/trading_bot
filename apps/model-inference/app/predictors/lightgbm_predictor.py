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


def predict(
    artifact_bytes: bytes,
    instances: list,
    model_kind: str,
    horizon: str,
    header: dict | None = None,
) -> list[Forecast]:
    X, objective = base.build_matrix(instances, header)

    booster = _load_booster(artifact_bytes)
    raw = np.atleast_1d(np.asarray(booster.predict(X), dtype=float))

    if objective == "regression":
        return [base.to_forecast_return(float(v), horizon) for v in raw]
    return [base.to_forecast(float(p), horizon) for p in raw]
