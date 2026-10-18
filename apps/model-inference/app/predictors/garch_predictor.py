"""GARCH-family predictor (I-1.11, N-4.2).

Serves the *conditional* one-step sigma: the model's variance recursion is
filtered through the recent bars the caller supplies and the next-step sigma is
returned. There is no unconditional fallback: missing, too few or non-finite
bars, and legacy bundles that carry only an unconditional sigma, raise
``base.Abstain(reason)``.

The recursion mirrors ``apps/model-trainer/app/families/vol.py``
(``variance_path``, ``conditional_sigma``, ``serve_from_state``); it is
duplicated rather than shared because trainer and inference are separate
services (as ``bundle.py`` is).

Each instance carries ``series["recent_returns"]`` (oldest first; ``features`` is also read) and,
for realised GARCH, ``recent_realized`` (same length). The platform fills them from the bar
store at call time.
"""

import math

import numpy as np

from ..schemas import Forecast
from . import base

SCALE = 100.0
DEFAULT_MIN_BARS = 30


def _eabs_t(nu: float) -> float:
    from scipy import special  # type: ignore

    return 2.0 * math.sqrt(nu - 2.0) * math.exp(
        special.gammaln((nu + 1.0) / 2.0) - special.gammaln(nu / 2.0)
    ) / (math.sqrt(math.pi) * (nu - 1.0))


def _next_variance(kind: str, p: dict, e: np.ndarray, xr, init: float) -> float:
    """One-step-ahead variance (percent^2) after filtering bars ``e`` (percent units)."""
    s2 = init
    if kind in ("garch", "gjr"):
        g = p.get("gamma", 0.0) if kind == "gjr" else 0.0
        for x in e:
            s2 = p["omega"] + (p["alpha"] + g * (x < 0)) * x * x + p["beta"] * s2
        return float(s2)
    if kind == "egarch":
        ea = _eabs_t(p["nu"])
        ls = math.log(init)
        for x in e:
            z = x / math.sqrt(math.exp(ls))
            ls = p["omega"] + p["beta"] * ls + p["alpha"] * (abs(z) - ea) + p["gamma"] * z
            ls = min(max(ls, -50.0), 50.0)
        return float(math.exp(ls))
    if kind == "realized_garch":
        lx = np.log(np.maximum(xr, 1e-12))
        lh = math.log(init)
        for x in lx:
            lh = p["omega"] + p["beta"] * lh + p["gamma"] * x
            lh = min(max(lh, -50.0), 50.0)
        return float(math.exp(lh))
    raise base.Abstain(f"unknown_vol_kind: {kind}")


def conditional_sigma(state: dict, recent_returns, recent_realized=None) -> float:
    if recent_returns is None:
        raise base.Abstain("no_recent_bars")
    try:
        r = np.asarray(recent_returns, float)
    except (TypeError, ValueError):
        raise base.Abstain("non_finite_recent_bars")
    need = int(state.get("min_bars", DEFAULT_MIN_BARS))
    if r.ndim != 1 or len(r) < need:
        raise base.Abstain(f"insufficient_recent_bars: have {len(r) if r.ndim == 1 else 0}, need {need}")
    if not np.all(np.isfinite(r)):
        raise base.Abstain("non_finite_recent_bars")
    xr = None
    if state["kind"] == "realized_garch":
        if recent_realized is None:
            raise base.Abstain("no_realized_measure")
        xr = np.asarray(recent_realized, float) * SCALE**2
        if len(xr) != len(r) or not np.all(np.isfinite(xr)):
            raise base.Abstain("bad_realized_measure")
    e = (r - state["mu"]) * SCALE
    return math.sqrt(_next_variance(state["kind"], state["params"], e, xr, state["init_var"])) / SCALE


def serve_from_state(state: dict, recent_returns, recent_realized=None) -> dict:
    """``{"abstain": False, "sigma": x}`` or ``{"abstain": True, "reason": "..."}``."""
    try:
        return {"abstain": False, "sigma": conditional_sigma(state, recent_returns, recent_realized)}
    except base.Abstain as a:
        return {"abstain": True, "reason": a.reason}


def _state(inner: bytes) -> tuple[dict, list[float] | None]:
    import pickle

    payload = pickle.loads(inner)
    if "state" not in payload:
        raise base.Abstain("legacy_garch_bundle: retrain; it carries only an unconditional sigma")
    return payload["state"], payload.get("levels")


def _levels(levels, header) -> list[float]:
    return list(levels or (header or {}).get("quantile_levels") or
                [0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95])


def _z(state: dict, levels: list[float]) -> np.ndarray:
    from scipy import stats  # type: ignore

    if state["kind"] == "realized_garch" or not state.get("nu"):
        z = stats.norm.ppf(levels)
    else:
        nu = float(state["nu"])
        z = stats.t.ppf(levels, df=nu) * math.sqrt((nu - 2) / nu)
    return np.sort(np.asarray(z, float))


def _features(inst) -> dict:
    """The instance's features overlaid on its named series (a series wins: it is the window)."""
    f = getattr(inst, "features", None)
    s = getattr(inst, "series", None)
    if isinstance(inst, dict):
        f = inst.get("features", {}) if f is None else f
        s = inst.get("series", {}) if s is None else s
    return {**(f or {}), **(s or {})}


def serve_rows(inner: bytes, header: dict | None, instances: list) -> list[dict]:
    """Per-instance ``{"abstain", "sigma"|"reason", "mu", "levels", "z"}``."""
    state, lv = _state(inner)
    levels = _levels(lv, header)
    z = _z(state, levels)
    out = []
    for inst in instances:
        f = _features(inst)
        res = serve_from_state(state, f.get("recent_returns"), f.get("recent_realized"))
        if not res["abstain"]:
            res.update(mu=float(state["mu"]), levels=levels, z=z)
        out.append(res)
    return out


def predict(
    artifact_bytes: bytes,
    instances: list,
    model_kind: str,
    horizon: str,
    header: dict | None = None,
) -> list[Forecast]:
    if len(instances) == 0:
        return []
    forecasts = []
    for i, r in enumerate(serve_rows(artifact_bytes, header, instances)):
        if r["abstain"]:
            raise base.Abstain(f"instance {i}: {r['reason']}")
        forecasts.append(
            base.to_distribution_forecast(r["z"], r["levels"], r["sigma"], horizon, mu=r["mu"])
        )
    return forecasts
