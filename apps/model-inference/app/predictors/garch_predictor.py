"""GARCH-t predictor (I-1.11).

Loads the GARCH parameters from the bundle and emits a distributional
volatility forecast using the unconditional variance as the σ estimate.
"""

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
    from scipy import stats  # type: ignore

    payload = pickle.loads(artifact_bytes)
    sigma_uncond: float = float(payload.get("sigma_uncond", 1e-3))
    nu: float = float(payload.get("nu", 10.0))
    levels: list[float] = payload.get("levels") or (header or {}).get("quantile_levels") or [
        0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95
    ]

    if len(instances) == 0:
        return []

    # σ-unit quantiles from Student-t(nu) distribution.
    q_sigma = np.array(stats.t.ppf(levels, df=nu), dtype=float)
    q_sigma_sorted = np.sort(q_sigma)

    return [
        base.to_distribution_forecast(q_sigma_sorted, levels, sigma_uncond, horizon)
        for _ in instances
    ]
