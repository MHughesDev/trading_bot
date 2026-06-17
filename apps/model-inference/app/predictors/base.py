from decimal import Decimal

import numpy as np

from ..schemas import Forecast


# --------------------------------------------------------------------------- #
# Distribution contract validation (I-1.12, serve side)
# --------------------------------------------------------------------------- #

def validate_distribution_serve(quantiles: np.ndarray, levels: list[float], sigma: float) -> bool:
    """Return True if the distribution output is valid; log and return False otherwise."""
    import logging
    log = logging.getLogger(__name__)
    n = len(levels)
    if n == 0 or quantiles.shape[-1] != n or sigma <= 0:
        log.warning("distribution contract violation: lengths or sigma invalid")
        return False
    for i in range(1, n):
        if levels[i] <= levels[i - 1]:
            log.warning("distribution contract violation: levels not sorted")
            return False
    for row in np.atleast_2d(quantiles):
        if not np.all(np.isfinite(row)):
            log.warning("distribution contract violation: non-finite quantiles")
            return False
        if not np.all(np.diff(row) >= 0):
            log.warning("distribution contract violation: quantiles not monotone")
            return False
    return True


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


def to_forecast_return(ret: float, horizon: str, scale: float = 0.01) -> Forecast:
    """Map a predicted forward *return* (not a probability) to a Forecast.

      - direction: up if ret > 0, down if ret < 0, flat near zero
      - confidence: |ret| / scale, clamped to [0, 1]
      - magnitude: decimal string of the predicted return
    """
    try:
        r = float(ret)
    except Exception:  # noqa: BLE001
        r = 0.0

    if r > 1e-6:
        direction = "up"
    elif r < -1e-6:
        direction = "down"
    else:
        direction = "flat"

    confidence = min(1.0, abs(r) / scale) if scale > 0 else 0.0
    confidence = max(0.0, confidence)

    magnitude = str(Decimal(str(r)).quantize(Decimal("0.000001")))

    return Forecast(
        direction=direction,
        magnitude=magnitude,
        confidence=confidence,
        horizon=horizon,
    )


def to_distribution_forecast(
    q_sigma: np.ndarray,
    levels: list[float],
    sigma: float,
    horizon: str,
) -> Forecast:
    """Build a distributional Forecast from σ-unit quantiles (I-1.11).

    - Rescales σ-units → return units
    - Derives point view (direction, magnitude, confidence) from the median
    - Returns point-only Forecast if validation fails (never crash on serve)
    """
    q_return = q_sigma * sigma
    # Median: interpolate to level 0.5
    median = float(np.interp(0.5, levels, q_return))

    if not validate_distribution_serve(q_sigma.reshape(1, -1), levels, sigma):
        return to_forecast_return(median, horizon)

    direction = "up" if median > 1e-8 else ("down" if median < -1e-8 else "flat")
    conf = _interval_confidence(q_return, levels, sigma)
    magnitude = str(Decimal(str(median)).quantize(Decimal("0.000001")))

    return Forecast(
        direction=direction,
        magnitude=magnitude,
        confidence=conf,
        horizon=horizon,
        quantile_levels=list(levels),
        quantiles_return=[float(v) for v in q_return],
        median_return=median,
        sigma=float(sigma),
    )


def _interval_confidence(q_return: np.ndarray, levels: list[float], sigma: float) -> float:
    q10 = float(np.interp(0.1, levels, q_return))
    q90 = float(np.interp(0.9, levels, q_return))
    spread = abs(q90 - q10)
    denom = 2.0 * sigma
    if denom <= 0:
        return 0.0
    return float(np.clip(1.0 - spread / denom, 0.0, 1.0))


def apply_scaler(X: np.ndarray, scaler) -> np.ndarray:
    """Standardize X with a persisted ``{"mean": [...], "std": [...]}`` scaler."""
    if not scaler or X.size == 0:
        return X
    try:
        mean = np.asarray(scaler.get("mean"), dtype=float)
        std = np.asarray(scaler.get("std"), dtype=float)
    except Exception:  # noqa: BLE001
        return X
    if mean.shape[0] != X.shape[1] or std.shape[0] != X.shape[1]:
        return X  # width mismatch — skip rather than corrupt
    std = np.where(std == 0.0, 1.0, std)
    return (X - mean) / std


def resolve_features(instances, header):
    """Return (feature_order, scaler) for a prediction, honoring the bundle.

    With a bundle header, the feature order and scaler are taken from training so
    columns line up exactly. Without one (bare artifact), fall back to the legacy
    sorted-key order and no scaling.
    """
    if header is not None and header.get("feature_order"):
        return list(header["feature_order"]), header.get("scaler")
    keys = set()
    for inst in instances:
        f = getattr(inst, "features", None)
        if f is None and isinstance(inst, dict):
            f = inst.get("features", {})
        keys.update((f or {}).keys())
    return sorted(keys), None


def build_matrix(instances, header) -> tuple[np.ndarray, str]:
    """Build the (scaled) feature matrix and return it with the objective."""
    feature_order, scaler = resolve_features(instances, header)
    X = features_matrix(instances, feature_order=feature_order)
    X = apply_scaler(X, scaler)
    objective = (header or {}).get("objective", "classification")
    return X.astype(np.float32), objective


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
