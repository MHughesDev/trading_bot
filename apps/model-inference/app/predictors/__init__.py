"""Predictor dispatch.

A trained artifact is either a self-describing *bundle* (new format, carries the
framework + feature order + scaler + objective) or a bare legacy artifact. For
bundles we dispatch straight to the matching framework predictor; for bare
artifacts we keep the tolerant "try each predictor" fallback so already-trained
models keep serving.
"""

from . import (
    base,
    bundle as bundle_mod,
    lightgbm_predictor,
    sklearn_predictor,
    torch_predictor,
    xgboost_predictor,
)

_BY_FRAMEWORK = {
    "xgboost": xgboost_predictor.predict,
    "lightgbm": lightgbm_predictor.predict,
    "sklearn": sklearn_predictor.predict,
    "torch": torch_predictor.predict,
}

_FALLBACK_ORDER = [
    xgboost_predictor.predict,
    lightgbm_predictor.predict,
    sklearn_predictor.predict,
    torch_predictor.predict,
]


def run_predict(artifact_bytes: bytes, instances: list, model_kind: str, horizon: str):
    """Return forecasts for the artifact, or None if nothing could decode it."""
    header, inner = bundle_mod.read_bundle(artifact_bytes)

    if header is not None:
        fn = _BY_FRAMEWORK.get(header.get("framework"))
        candidates = [fn] if fn is not None else list(_FALLBACK_ORDER)
        for predict in candidates:
            if predict is None:
                continue
            try:
                result = predict(inner, instances, model_kind, horizon, header)
                if result is not None:
                    return result
            except Exception:
                continue

    # Bare/legacy artifact (or bundle that failed every framework): try each
    # predictor against the raw bytes with no bundle metadata.
    for predict in _FALLBACK_ORDER:
        try:
            result = predict(artifact_bytes, instances, model_kind, horizon, None)
            if result is not None:
                return result
        except Exception:
            continue
    return None
