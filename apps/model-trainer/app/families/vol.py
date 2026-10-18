"""Volatility family: GARCH, GJR, EGARCH, realised GARCH (N-2.3) and the one
conditional-sigma serving function (N-4.2).

Own maximum likelihood in numpy/scipy (`arch` is not installed and not needed).
Everything is fitted in percent units (returns x 100) for numerical conditioning;
`serve`/`conditional_sigma` return return-units sigma.

Serving contract (N-4.2, MD-12, FEAT-006 section 8)
---------------------------------------------------
`conditional_sigma(state, recent_returns, recent_realized=None)` filters the model's variance
recursion through the recent bars and returns the one-step-ahead conditional sigma. If the bars
are absent, too few, or non-finite it raises `Abstain(reason)`. There is deliberately **no code
path that returns the unconditional sigma**: `sigma_uncond` exists in the state only as a
diagnostic and no serving function reads it (a test greps for that).

The inference sidecar (apps/model-inference, outside the trainer) must call
`serve_from_state(state, returns, realized)` (or `conditional_sigma`) with the recent bars window
and turn an `Abstain` into an abstain response with its `reason`.
"""

from __future__ import annotations

import math
import pickle
import warnings

import numpy as np
import pandas as pd
from scipy import optimize, special, stats
from scipy.signal import lfilter

from .. import engine, failures

SCALE = 100.0
DEFAULT_MIN_BARS = 30
RETURN_COLUMNS = ("ret", "return", "log_return", "logret", "ret_1", "returns")
KINDS = ("garch", "gjr", "egarch", "realized_garch")


class Abstain(failures.TrainerFailure):
    """The model declines to forecast; `reason` says why. Never a fallback value."""

    terminal = failures.DATA_ERROR

    def __init__(self, reason: str):
        super().__init__(reason)
        self.reason = reason


# --------------------------------------------------------------------------- #
# Variance recursions (shared by the likelihood and by serving)
# --------------------------------------------------------------------------- #
def _eabs_t(nu: float) -> float:
    """E|z| for a unit-variance Student-t."""
    return 2.0 * math.sqrt(nu - 2.0) * math.exp(
        special.gammaln((nu + 1.0) / 2.0) - special.gammaln(nu / 2.0)
    ) / (math.sqrt(math.pi) * (nu - 1.0))


def variance_path(kind: str, p: dict, e: np.ndarray, xr: np.ndarray | None, init: float):
    """Return `(s2, s2_next)` in percent^2. `s2[t]` is the variance of bar t given bars < t."""
    n = len(e)
    if kind in ("garch", "gjr"):
        g = p.get("gamma", 0.0) if kind == "gjr" else 0.0
        u = p["omega"] + (p["alpha"] + g * (e[:-1] < 0)) * e[:-1] ** 2
        s2 = np.empty(n)
        s2[0] = init
        if n > 1:
            s2[1:], _ = lfilter([1.0], [1.0, -p["beta"]], u, zi=[p["beta"] * init])
        nxt = p["omega"] + (p["alpha"] + g * (e[-1] < 0)) * e[-1] ** 2 + p["beta"] * s2[-1]
        return s2, float(nxt)
    if kind == "egarch":
        nu = p["nu"]
        ea = _eabs_t(nu)
        om, al, ga, be = p["omega"], p["alpha"], p["gamma"], p["beta"]
        ls = math.log(init)
        s2 = np.empty(n)
        for t in range(n):
            s2[t] = math.exp(ls)
            z = e[t] / math.sqrt(s2[t])
            ls = om + be * ls + al * (abs(z) - ea) + ga * z
            ls = min(max(ls, -50.0), 50.0)
        return s2, float(math.exp(ls))
    if kind == "realized_garch":
        if xr is None:
            raise Abstain("no_realized_measure")
        lx = np.log(np.maximum(xr, 1e-12))
        u = p["omega"] + p["gamma"] * lx[:-1]
        lh = np.empty(n)
        lh[0] = math.log(init)
        if n > 1:
            lh[1:], _ = lfilter([1.0], [1.0, -p["beta"]], u, zi=[p["beta"] * lh[0]])
        nxt = p["omega"] + p["beta"] * lh[-1] + p["gamma"] * lx[-1]
        return np.exp(np.clip(lh, -50, 50)), float(math.exp(min(max(nxt, -50.0), 50.0)))
    raise ValueError(f"unknown vol kind {kind!r}")


def _t_ll(e: np.ndarray, s2: np.ndarray, nu: float) -> np.ndarray:
    c = special.gammaln((nu + 1) / 2) - special.gammaln(nu / 2) - 0.5 * math.log(math.pi * (nu - 2))
    return c - 0.5 * np.log(s2) - (nu + 1) / 2 * np.log1p(e**2 / (s2 * (nu - 2)))


# --------------------------------------------------------------------------- #
# Fitting
# --------------------------------------------------------------------------- #
_NAMES = {
    "garch": ["omega", "alpha", "beta", "nu"],
    "gjr": ["omega", "alpha", "gamma", "beta", "nu"],
    "egarch": ["omega", "alpha", "gamma", "beta", "nu"],
    "realized_garch": ["omega", "beta", "gamma", "xi", "phi", "tau1", "tau2", "sig_u"],
}


def _unpack(kind, th):
    return dict(zip(_NAMES[kind], (float(v) for v in th)))


def _nll(th, kind, e, xr, init):
    p = _unpack(kind, th)
    try:
        if kind in ("garch", "gjr"):
            g = p.get("gamma", 0.0) if kind == "gjr" else 0.0
            if p["alpha"] + p["beta"] + 0.5 * g >= 0.9999:
                return 1e10
        if kind == "egarch" and abs(p["beta"]) >= 0.9999:
            return 1e10
        if kind == "realized_garch" and abs(p["beta"] + p["gamma"] * p["phi"]) >= 0.9999:
            return 1e10
        s2, _ = variance_path(kind, p, e, xr, init)
        if not np.all(np.isfinite(s2)) or np.any(s2 <= 0):
            return 1e10
        if kind == "realized_garch":
            z = e / np.sqrt(s2)
            lx = np.log(np.maximum(xr, 1e-12))
            u = lx - p["xi"] - p["phi"] * np.log(s2) - p["tau1"] * z - p["tau2"] * (z**2 - 1)
            su2 = p["sig_u"] ** 2
            ll = -0.5 * (np.log(2 * np.pi * s2) + e**2 / s2) - 0.5 * (np.log(2 * np.pi * su2) + u**2 / su2)
        else:
            ll = _t_ll(e, s2, p["nu"])
        v = -float(np.sum(ll))
        return v if np.isfinite(v) else 1e10
    except (FloatingPointError, ValueError, OverflowError):
        return 1e10


def _starts_bounds(kind, var):
    lv = math.log(var)
    if kind == "garch":
        b = [(1e-6, 50 * var), (1e-4, 0.5), (0.3, 0.9998), (2.5, 100.0)]
        st = [[var * 0.05, 0.05, 0.9, 8.0], [var * 0.15, 0.10, 0.8, 6.0]]
    elif kind == "gjr":
        b = [(1e-6, 50 * var), (1e-4, 0.5), (-0.2, 0.6), (0.3, 0.9998), (2.5, 100.0)]
        st = [[var * 0.05, 0.03, 0.06, 0.9, 8.0], [var * 0.15, 0.05, 0.10, 0.8, 6.0]]
    elif kind == "egarch":
        b = [(-5.0, 5.0), (0.0, 1.0), (-0.5, 0.5), (0.3, 0.9998), (2.5, 100.0)]
        st = [[lv * 0.05, 0.10, -0.05, 0.95, 8.0], [lv * 0.1, 0.15, -0.02, 0.9, 6.0]]
    else:
        b = [(-5.0, 5.0), (0.0, 0.9998), (0.0, 2.0), (-5.0, 5.0), (0.1, 3.0), (-0.5, 0.5), (-0.5, 0.5), (0.01, 3.0)]
        st = [[0.05, 0.6, 0.35, -0.1, 1.0, -0.05, 0.05, 0.4]]
    return st, b


def fit(kind: str, returns: np.ndarray, realized: np.ndarray | None = None,
        *, min_bars: int = DEFAULT_MIN_BARS) -> dict:
    """Fit by MLE. `returns` in return units; `realized` (realised variance, return^2 units) for
    realised GARCH. Returns a picklable state."""
    if kind not in KINDS:
        raise ValueError(f"unknown vol kind {kind!r}")
    r = np.asarray(returns, float)
    ok = np.isfinite(r)
    if kind == "realized_garch":
        if realized is None:
            raise failures.DataError("realized_garch requires a realised variance column")
        xr = np.asarray(realized, float) * SCALE**2
        ok &= np.isfinite(xr) & (xr > 0)
        xr = xr[ok]
    else:
        xr = None
    r = r[ok]
    if len(r) < 100:
        raise failures.DataError(f"{kind} needs at least 100 return observations; got {len(r)}")
    mu = float(r.mean())
    e = (r - mu) * SCALE
    var = float(np.var(e))
    if var <= 0:
        raise failures.DataError("constant return series")
    starts, bounds = _starts_bounds(kind, var)
    best = None
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        for s0 in starts:
            res = optimize.minimize(_nll, np.array(s0, float), args=(kind, e, xr, var),
                                    method="L-BFGS-B", bounds=bounds, options={"maxiter": 400})
            if best is None or res.fun < best.fun:
                best = res
    if best is None or best.fun >= 1e9:
        raise failures.NanDivergence(f"{kind} MLE found no valid parameters")
    p = _unpack(kind, best.x)
    s2, _ = variance_path(kind, p, e, xr, var)
    return {
        "kind": kind, "mu": mu, "scale": SCALE, "params": p, "nu": p.get("nu"),
        "init_var": var, "min_bars": int(min_bars),
        "loglik": -float(best.fun), "converged": bool(best.success), "n_obs": int(len(e)),
        # Diagnostic ONLY. No serving function reads this key.
        "sigma_uncond": float(math.sqrt(float(np.mean(s2))) / SCALE),
    }


# --------------------------------------------------------------------------- #
# Serving: conditional sigma or abstain. No unconditional fallback exists.
# --------------------------------------------------------------------------- #
def conditional_sigma(state: dict, recent_returns, recent_realized=None) -> float:
    if recent_returns is None:
        raise Abstain("no_recent_bars")
    r = np.asarray(recent_returns, float)
    need = int(state.get("min_bars", DEFAULT_MIN_BARS))
    if r.ndim != 1 or len(r) < need:
        raise Abstain(f"insufficient_recent_bars: have {len(r)}, need {need}")
    if not np.all(np.isfinite(r)):
        raise Abstain("non_finite_recent_bars")
    xr = None
    if state["kind"] == "realized_garch":
        if recent_realized is None:
            raise Abstain("no_realized_measure")
        xr = np.asarray(recent_realized, float) * SCALE**2
        if len(xr) != len(r) or not np.all(np.isfinite(xr)):
            raise Abstain("bad_realized_measure")
    e = (r - state["mu"]) * SCALE
    _, s2_next = variance_path(state["kind"], state["params"], e, xr, state["init_var"])
    return float(math.sqrt(s2_next) / SCALE)


def serve_from_state(state: dict, recent_returns, recent_realized=None) -> dict:
    """`{"abstain": False, "sigma": x}` or `{"abstain": True, "reason": "..."}`."""
    try:
        return {"abstain": False, "sigma": conditional_sigma(state, recent_returns, recent_realized)}
    except Abstain as a:
        return {"abstain": True, "reason": a.reason}


def sigma_path(state: dict, returns: np.ndarray, realized: np.ndarray | None = None) -> np.ndarray:
    """Per-row one-step-ahead sigma: element t uses bars <= t only. Warm-up rows are NaN."""
    r = np.asarray(returns, float)
    need = int(state.get("min_bars", DEFAULT_MIN_BARS))
    out = np.full(len(r), np.nan)
    if len(r) < need or not np.all(np.isfinite(r)):
        raise Abstain(f"insufficient_recent_bars: have {len(r)}, need {need}")
    e = (r - state["mu"]) * SCALE
    xr = None if realized is None else np.asarray(realized, float) * SCALE**2
    s2, _ = variance_path(state["kind"], state["params"], e, xr, state["init_var"])
    # s2[t] is the variance OF bar t; the forecast for bar t+1 made at t is s2[t+1].
    nxt = np.empty(len(r))
    nxt[:-1] = s2[1:]
    _, last = variance_path(state["kind"], state["params"], e, xr, state["init_var"])
    nxt[-1] = last
    out[need - 1:] = np.sqrt(nxt[need - 1:]) / SCALE
    return out


def quantiles(state: dict, sigma: np.ndarray, levels: list[float]) -> np.ndarray:
    """(N, L) return-unit quantiles; unit-variance Student-t (Gaussian for realised GARCH)."""
    if state["kind"] == "realized_garch" or not state.get("nu"):
        z = stats.norm.ppf(levels)
    else:
        nu = float(state["nu"])
        z = stats.t.ppf(levels, df=nu) * math.sqrt((nu - 2) / nu)
    return state["mu"] + np.asarray(sigma)[:, None] * z[None, :]


# --------------------------------------------------------------------------- #
# Frame plumbing and registry entry points
# --------------------------------------------------------------------------- #
def returns_from_frame(definition: dict, df: pd.DataFrame, source: str | None = None):
    """`(returns, realized_or_None, source_name)`. Source order: explicit
    `definition["vol"]["returns_column"]`, a known return column, log-diff of `close`, else the
    label when the target is a forward return."""
    vol_cfg = definition.get("vol") or {}
    col = source or vol_cfg.get("returns_column")
    realized = df["realized_variance"].to_numpy(float) if "realized_variance" in df.columns else None
    if col and col in df.columns:
        return df[col].to_numpy(float), realized, col
    if col == "close" or (not col and not any(c in df.columns for c in RETURN_COLUMNS) and "close" in df.columns):
        c = df["close"].to_numpy(float)
        r = np.concatenate([[0.0], np.diff(np.log(np.maximum(c, 1e-12)))])
        return r, realized, "close"
    for c in RETURN_COLUMNS:
        if c in df.columns:
            return df[c].to_numpy(float), realized, c
    from .baseline import target_kind

    if target_kind(definition) == "forward_return" and "label" in df.columns:
        return df["label"].to_numpy(float), realized, "label"
    raise failures.DataError("no return series: provide a return column, `close`, or a forward_return label")


def _train(kind: str, definition: dict, df: pd.DataFrame, emit):
    engine.seed_everything(definition)
    r, xr, src = returns_from_frame(definition, df)
    wf = definition.get("_wf_fold")
    if wf:
        tr, te = slice(wf["train_start"], wf["train_end"]), slice(wf["test_start"], wf["test_end"])
    else:
        tr, _va, te = engine.split_indices(len(df), embargo=int(definition.get("_embargo_bars", 0) or 0))
    levels = list(((definition.get("output") or {}).get("quantile_levels")) or
                  [0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95])
    min_bars = int(((definition.get("vol") or {}).get("min_bars")) or DEFAULT_MIN_BARS)
    emit("fitting", 10.0, {"status": f"fitting_{kind}"})
    state = fit(kind, r[tr], None if xr is None else xr[tr], min_bars=min_bars)
    state["returns_source"] = src
    emit("fitting", 70.0, {"loglik": state["loglik"]})
    extra = {"arch": kind, "returns_source": src, "loglik": state["loglik"],
             "converged": state["converged"], "sigma_uncond": state["sigma_uncond"],
             "serving": "conditional_sigma_or_abstain", **{f"param_{k}": v for k, v in state["params"].items()}}
    n_test = 0
    if te.stop and te.stop > te.start and len(r[te]):
        try:
            sp = sigma_path(state, r, xr if kind == "realized_garch" else None)[te]
            ok = np.isfinite(sp) & np.isfinite(r[te])
            if ok.any():
                q = quantiles(state, sp[ok], levels)
                crps = engine.pinball_loss(r[te][ok], q, levels)
                extra.update(test_crps=crps, crps=crps,
                             test_nll=float(-np.mean(_t_ll((r[te][ok] - state["mu"]) * SCALE,
                                                           (sp[ok] * SCALE) ** 2, state["nu"] or 1e6 + 3))) if state["nu"] else None)
                n_test = int(ok.sum())
        except Abstain:
            pass
    metrics = engine.assemble_metrics("regression", [], framework_version="vol-mle-1",
                                      n_train=len(r[tr]), n_val=0, n_test=n_test, extra=extra)
    bundle = engine.wrap_bundle(
        pickle.dumps({"state": state, "levels": levels}),
        framework="garch", objective="quantile", feature_order=[], scaler=None,
        output_kind="distribution", quantile_levels=levels, sigma_scaler=None,
        extra={"arch_type": kind, "family": "vol", "variant": kind, "serving": "conditional_sigma",
               "min_bars": min_bars, "returns_source": src},
    )
    emit("fitting", 100.0, metrics)
    return bundle, metrics


def train_garch(definition, df, emit):
    return _train("garch", definition, df, emit)


def train_gjr(definition, df, emit):
    return _train("gjr", definition, df, emit)


def train_egarch(definition, df, emit):
    return _train("egarch", definition, df, emit)


def train_realized_garch(definition, df, emit):
    return _train("realized_garch", definition, df, emit)


def state_from_bundle_inner(inner: bytes) -> dict:
    payload = pickle.loads(inner)
    if "state" not in payload:
        raise failures.DataError(
            "legacy GARCH bundle (unconditional-sigma only): retrain; it cannot be served"
        )
    return payload["state"]
