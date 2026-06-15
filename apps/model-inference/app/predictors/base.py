from decimal import Decimal

import numpy as np

from ..schemas import Forecast


def predict(artifact_bytes: bytes, instances: list, model_kind: str, horizon: str) -> list[Forecast]:
    """Abstract predictor signature.

    Concrete predictor modules (xgboost_predictor, lightgbm_predictor, ...)
    implement a module-level ``predict`` with this exact signature.
    """
    raise NotImplementedError


def to_forecast(score: float, horizon: str) -> Forecast:
    """Map a probability/score in [0,1] (or a raw return) to a Forecast.

    Deterministic mapping:
      - direction: up if score > 0.55, down if score < 0.45, else flat
      - confidence: abs(score - 0.5) * 2, clamped to [0, 1]
      - magnitude: decimal string of (score - 0.5)
    """
    try:
        s = float(score)
    except Exception:
        s = 0.5

    if s > 0.55:
        direction = "up"
    elif s < 0.45:
        direction = "down"
    else:
        direction = "flat"

    confidence = abs(s - 0.5) * 2.0
    confidence = max(0.0, min(1.0, confidence))

    magnitude = str(Decimal(str(s - 0.5)).quantize(Decimal("0.000001")))

    return Forecast(
        direction=direction,
        magnitude=magnitude,
        confidence=confidence,
        horizon=horizon,
    )


def features_matrix(instances, feature_order=None) -> np.ndarray:
    """Build a 2D numpy array from instance.features dicts.

    If ``feature_order`` is given, columns follow that order; otherwise
    feature keys are sorted for determinism (using the union of keys
    across all instances).
    """
    feats = []
    for inst in instances:
        f = getattr(inst, "features", None)
        if f is None and isinstance(inst, dict):
            f = inst.get("features", {})
        feats.append(f or {})

    if feature_order is None:
        keys = set()
        for f in feats:
            keys.update(f.keys())
        feature_order = sorted(keys)

    rows = []
    for f in feats:
        rows.append([float(f.get(k, 0.0)) for k in feature_order])

    if not rows:
        return np.zeros((0, len(feature_order)), dtype=np.float32)
    return np.asarray(rows, dtype=np.float32)
