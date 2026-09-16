"""Shared helpers for templated tools (AGENT-006 §3.3; Set S tasks S-3.1, S-3.2).

This file is **prepended** to every template body before the runner sees it.
Templates cannot import it: the runner is handed one source string on stdin and
shares no import path with the workspace. Concatenation is the cost of that
isolation, and `skills::template::Template::source_line` maps a traceback line
back past this preamble to the file the author actually edited.

Everything here exists because more than one template needs it and two copies of
a block bootstrap are two block bootstraps that can disagree. Nothing here
reaches the network or the database; a template's rows arrive as parquet.

Determinism
-----------
`detail` must be byte-identical across two seeded runs, and there are three ways
for that to fail that have nothing to do with the statistics:

1. **Thread count.** A BLAS reduction over N threads sums in N-dependent order,
   and floating-point addition is not associative. The thread caps below are set
   *before* numpy is imported, which is the only point at which they are read.
2. **Hash order.** Iterating a set or a dict built from one gives an order that
   depends on the process hash seed. `sanitise` sorts every mapping it emits.
3. **The last bits of a float.** Even single-threaded, a fused multiply-add on
   one machine and not another changes the final ulp. `sanitise` rounds to
   twelve significant digits, which is far beyond any statistic's precision here
   and well inside a double's fifteen.

NaN is not a number and not JSON
--------------------------------
`json.dumps` writes bare `NaN`, `Infinity` and `-Infinity`, none of which are
valid JSON, and a strict parser on the other side rejects the whole envelope —
so one undefined skewness loses an entire result. `sanitise` maps all three to
`null`, which the summary renderer already knows how to print as "(n unknown)".
"""

from __future__ import annotations

import os as _os

# Before numpy. See "Determinism" above.
for _var in (
    "OMP_NUM_THREADS",
    "OPENBLAS_NUM_THREADS",
    "MKL_NUM_THREADS",
    "NUMEXPR_NUM_THREADS",
    "VECLIB_MAXIMUM_THREADS",
):
    _os.environ[_var] = "1"

import math
import numpy as np
import pandas as pd

try:
    import scipy
    import scipy.stats as st
    from scipy import optimize as _opt

    _HAVE_SCIPY = True
except ImportError:  # pragma: no cover - the image installs it
    _HAVE_SCIPY = False
    scipy = None
    st = None
    _opt = None


# ── Provenance ───────────────────────────────────────────────────────────────


def provenance() -> dict:
    """Which libraries produced this number.

    A statistic is only reproducible against the version that computed it: the
    default `ddof`, the p-value approximation and the optimiser's convergence
    criterion have all changed across minor releases of these packages. A result
    kept as evidence without its provenance is a result nobody can re-derive.
    """
    return {
        "python": ".".join(str(v) for v in __import__("sys").version_info[:3]),
        "numpy": np.__version__,
        "pandas": pd.__version__,
        "scipy": getattr(scipy, "__version__", None),
    }


# ── JSON that a strict parser will take back ─────────────────────────────────

_SIGNIFICANT_DIGITS = 12


def _round(x: float) -> float:
    if x == 0.0 or not math.isfinite(x):
        return x
    magnitude = math.floor(math.log10(abs(x)))
    return round(x, _SIGNIFICANT_DIGITS - 1 - magnitude)


def sanitise(obj):
    """Make `obj` serialisable, ordered and finite.

    Recursive by necessity: a NaN four levels down in a nested diagnostic breaks
    the same envelope a NaN at the top would.
    """
    if obj is None or isinstance(obj, (bool, str)):
        return obj
    if isinstance(obj, (int, np.integer)):
        return int(obj)
    if isinstance(obj, (float, np.floating)):
        v = float(obj)
        return _round(v) if math.isfinite(v) else None
    if isinstance(obj, np.ndarray):
        return [sanitise(v) for v in obj.tolist()]
    if isinstance(obj, (list, tuple)):
        return [sanitise(v) for v in obj]
    if isinstance(obj, dict):
        # Sorted: see "Hash order" above.
        return {str(k): sanitise(obj[k]) for k in sorted(obj, key=str)}
    if isinstance(obj, pd.Timestamp):
        return obj.isoformat()
    if isinstance(obj, pd.Series):
        return sanitise(obj.to_dict())
    if hasattr(obj, "item"):
        return sanitise(obj.item())
    return str(obj)


def result(detail: dict, n=None, se=None, ids=None) -> dict:
    """The one shape a template answers with.

    Mirrors `mcp_server_lib::spec::ToolResult`. There is no `summary` field:
    the summary is rendered on the Rust side from the header's template and
    these `detail` fields, so a template cannot write a sentence that disagrees
    with its own numbers (§2.4).
    """
    return {
        "n": None if n is None else int(n),
        "se": None if se is None else sanitise(float(se)),
        "ids": {str(k): str(v) for k, v in sorted((ids or {}).items())},
        "detail": sanitise(dict(detail, provenance=provenance())),
    }


def refuse(reason: str, **fields) -> dict:
    """Decline to answer, and say why.

    A tool that cannot compute its statistic returns this rather than a number
    with a caveat in prose: the caveat is not machine-readable and the number
    gets cited. `refused` is checked by the dispatcher before the summary is
    rendered.
    """
    return {
        "refused": True,
        "reason": reason,
        "detail": sanitise(dict(fields, provenance=provenance())),
    }


# ── Inputs ───────────────────────────────────────────────────────────────────


def load(payload: dict, role: str = "primary") -> pd.DataFrame:
    """The materialised series for `role`, as a time-indexed frame.

    The harness wrote this file (S-3.2) and its path is a hash of the window, so
    the same request reads the same bytes. The frame is sorted and de-duplicated
    here rather than trusted: a duplicate bar is the platform's known data defect
    (the 2026-08-13 duplicate in BTC-USD/coinbase/1m) and a statistic computed
    over it is wrong in a way no diagnostic would show.
    """
    data = payload.get("data") or {}
    path = data.get(role)
    if not path:
        raise ValueError(
            f"this template asked for the `{role}` series and the payload has no path for it; "
            f"the header's `series` list is what the harness materialises"
        )
    frame = pd.read_parquet(path)
    if "ts" in frame.columns:
        frame = frame.set_index("ts")
    frame = frame[~frame.index.duplicated(keep="first")].sort_index()
    return frame


def rng(payload: dict) -> np.random.Generator:
    """The run's generator, seeded from the call.

    `default_rng` rather than the legacy global: the global is process state, and
    two templates in one process would then depend on which ran first.
    """
    seed = payload.get("seed")
    if seed is None:
        raise ValueError(
            "every template that draws random numbers takes an explicit `seed`; an unseeded "
            "result cannot be reproduced and cannot be cited"
        )
    return np.random.default_rng(int(seed))


def epoch_seconds(index) -> np.ndarray:
    """Whole seconds since the epoch, whatever resolution the index carries.

    The trap this exists for, measured rather than imagined: a parquet round
    trip yields `datetime64[us]` on some writer/pandas combinations and
    `datetime64[ns]` on others, and `.view("int64")` hands back the raw count in
    whichever unit it happens to be. Dividing that by 1e9 "to get seconds" is
    correct for one of them and silently wrong by a factor of a thousand for the
    rest — which lands every timestamp in January 1970 and collapses every
    time bucket into one.

    It was found by `bar_boundary_check` reporting 720 of 720 hourly bars as
    violating, on a fixture built to contain no violation at all. A tool that
    detects a timestamp defect had one.

    Subtracting a `Timestamp` and dividing by a `Timedelta` is resolution-
    independent by construction, which is why it is done that way rather than
    by inspecting the dtype.
    """
    ts = pd.DatetimeIndex(index)
    if ts.tz is None:
        ts = ts.tz_localize("UTC")
    epoch = pd.Timestamp("1970-01-01T00:00:00Z")
    return np.asarray((ts - epoch) // pd.Timedelta(seconds=1), dtype="int64")


def close_returns(frame: pd.DataFrame, kind: str = "log") -> np.ndarray:
    """Bar returns from a close column, with non-positive prices dropped.

    A zero or negative close is a data defect, not a price; `log` of it is `-inf`
    and would poison every moment downstream.
    """
    close = pd.to_numeric(frame["close"], errors="coerce").to_numpy(dtype=float)
    close = close[np.isfinite(close) & (close > 0)]
    if close.size < 2:
        return np.empty(0, dtype=float)
    if kind == "simple":
        return close[1:] / close[:-1] - 1.0
    return np.diff(np.log(close))


# ── Statistics the catalogue shares ──────────────────────────────────────────


def nearest_rank(values, q: float) -> float:
    """The `q`-quantile by nearest rank — an observed value, never an average.

    Interpolating between two observations invents a number the sample does not
    contain, which matters for a percentile used as a threshold: a calibration
    factor that no run actually exhibited is a factor nothing validates.
    """
    x = np.sort(np.asarray(values, dtype=float))
    x = x[np.isfinite(x)]
    if x.size == 0:
        return float("nan")
    rank = max(1, math.ceil(q * x.size))
    return float(x[min(rank, x.size) - 1])


def autocorr(x: np.ndarray, lag: int) -> float:
    x = np.asarray(x, dtype=float)
    if x.size <= lag + 1:
        return float("nan")
    a = x[:-lag] - x.mean()
    b = x[lag:] - x.mean()
    denom = np.sum((x - x.mean()) ** 2)
    return float(np.sum(a * b) / denom) if denom > 0 else float("nan")


def optimal_block_length(x: np.ndarray) -> int:
    """A data-driven block length for the bootstrap (Politis & White, 2004).

    Derived rather than constant, which is S-3.9's acceptance criterion: a fixed
    block of 20 is right for one series and destroys the dependence structure of
    another. The estimate is the flat-top lag-window rule, simplified to the
    first lag whose autocorrelation falls inside the white-noise band and held
    for `K` further lags.
    """
    x = np.asarray(x, dtype=float)
    n = x.size
    if n < 32:
        return max(1, int(round(n ** (1.0 / 3.0))))
    band = 2.0 * math.sqrt(math.log10(n) / n)
    k = max(5, int(math.ceil(math.sqrt(math.log10(n)))))
    m = 0
    max_lag = min(n // 4, int(math.ceil(2.0 * math.sqrt(n))) + k)
    for lag in range(1, max_lag + 1):
        window = [abs(autocorr(x, lag + j)) for j in range(k) if lag + j <= max_lag]
        if window and max(window) < band:
            m = lag - 1
            break
    else:
        m = max_lag
    m = max(1, m)
    # b_opt ~ (2 * G^2 / D)^(1/3) * n^(1/3); the constant collapses to this form
    # for the circular rule, and clamping keeps it inside the sample.
    b = int(round((2.0 * m) ** (1.0 / 3.0) * n ** (1.0 / 3.0)))
    return int(min(max(1, b), max(1, n // 4)))


def moving_block_bootstrap(x: np.ndarray, n_boot: int, block: int, generator) -> np.ndarray:
    """`n_boot` resamples of `x` in overlapping blocks of `block`.

    Blocks rather than i.i.d. draws because returns are not independent: an
    i.i.d. bootstrap destroys volatility clustering and produces confidence
    bands far tighter than the truth, which is the failure mode that makes a
    noise strategy look significant.

    Returns an `(n_boot, len(x))` array.
    """
    x = np.asarray(x, dtype=float)
    n = x.size
    if n == 0:
        return np.empty((n_boot, 0), dtype=float)
    block = int(min(max(1, block), n))
    per = int(math.ceil(n / block))
    starts = generator.integers(0, n - block + 1, size=(n_boot, per))
    offsets = np.arange(block)
    idx = (starts[:, :, None] + offsets[None, None, :]).reshape(n_boot, per * block)
    return x[idx[:, :n]]


def benjamini_hochberg(pvalues, alpha: float = 0.05):
    """BH step-up. Returns `(adjusted, rejected)`, both in input order.

    The adjusted values are returned because a survivor reported without one is
    a survivor of an unstated number of tests — S-3.4's acceptance criterion
    says the correction is not optional, and returning only a boolean would let
    a caller report the raw p-value beside it.
    """
    p = np.asarray(pvalues, dtype=float)
    m = p.size
    if m == 0:
        return np.empty(0), np.empty(0, dtype=bool)
    order = np.argsort(p)
    ranked = p[order] * m / (np.arange(m) + 1)
    # Step-up: monotone from the largest down, so an adjusted value never
    # exceeds one computed at a larger rank.
    adjusted_sorted = np.minimum.accumulate(ranked[::-1])[::-1]
    adjusted = np.empty(m, dtype=float)
    adjusted[order] = np.minimum(adjusted_sorted, 1.0)
    return adjusted, adjusted <= alpha


def romano_wolf(statistics, bootstrap_statistics, alpha: float = 0.05):
    """Romano-Wolf stepdown on studentised statistics.

    Stronger than BH where it matters here: BH controls the false discovery
    *rate* under independence, and hypotheses over overlapping windows of one
    price series are about as dependent as hypotheses get. Romano-Wolf controls
    the family-wise error rate and takes the dependence from the bootstrap
    distribution itself rather than assuming it away.

    `statistics` is `(k,)`; `bootstrap_statistics` is `(n_boot, k)`, already
    centred under the null.  Returns `(adjusted, rejected)`.
    """
    t = np.abs(np.asarray(statistics, dtype=float))
    boot = np.abs(np.asarray(bootstrap_statistics, dtype=float))
    k = t.size
    adjusted = np.ones(k, dtype=float)
    remaining = list(range(k))
    order = list(np.argsort(-t))
    previous = 0.0
    for j in order:
        if not remaining:
            break
        maxima = boot[:, remaining].max(axis=1)
        p = float((np.sum(maxima >= t[j]) + 1) / (maxima.size + 1))
        # Monotone: a hypothesis tested later cannot get a smaller p-value than
        # one rejected before it.
        previous = max(previous, p)
        adjusted[j] = min(previous, 1.0)
        remaining.remove(j)
    return adjusted, adjusted <= alpha


def newey_west_se(x: np.ndarray, lags: int = None) -> float:
    """HAC standard error of the mean.

    The plain `s/sqrt(n)` assumes independence and understates the error on an
    autocorrelated series — which is exactly how a Sharpe ratio acquires a
    significance it does not have.
    """
    x = np.asarray(x, dtype=float)
    x = x[np.isfinite(x)]
    n = x.size
    if n < 3:
        return float("nan")
    if lags is None:
        lags = int(math.floor(4 * (n / 100.0) ** (2.0 / 9.0)))
    lags = int(max(0, min(lags, n - 2)))
    e = x - x.mean()
    gamma0 = float(np.dot(e, e) / n)
    total = gamma0
    for lag in range(1, lags + 1):
        weight = 1.0 - lag / (lags + 1.0)
        gamma = float(np.dot(e[:-lag], e[lag:]) / n)
        total += 2.0 * weight * gamma
    return math.sqrt(max(total, 0.0) / n)


def summary_stats(x: np.ndarray) -> dict:
    """Mean, dispersion and sample size, with the HAC error beside the naive one."""
    x = np.asarray(x, dtype=float)
    x = x[np.isfinite(x)]
    n = int(x.size)
    if n == 0:
        return {"n": 0, "mean": None, "sd": None, "se": None, "se_iid": None}
    sd = float(np.std(x, ddof=1)) if n > 1 else float("nan")
    return {
        "n": n,
        "mean": float(np.mean(x)),
        "sd": sd,
        "se": newey_west_se(x),
        "se_iid": sd / math.sqrt(n) if n > 1 else float("nan"),
    }


def max_drawdown(equity: np.ndarray) -> float:
    equity = np.asarray(equity, dtype=float)
    if equity.size == 0:
        return float("nan")
    peak = np.maximum.accumulate(equity)
    with np.errstate(divide="ignore", invalid="ignore"):
        dd = np.where(peak > 0, equity / peak - 1.0, np.nan)
    return float(np.nanmin(dd)) if np.isfinite(dd).any() else float("nan")


def bars_per_year(timeframe: str) -> float:
    """Annualisation factor for a timeframe string.

    Crypto trades continuously, so this is calendar time and not 252 trading
    days. A template annualising an equity series with a crypto factor (or the
    reverse) produces a Sharpe that is wrong by a constant — which is invisible
    unless the factor is stated, so it is returned in `detail`.
    """
    unit = timeframe[-1:].lower()
    try:
        size = int(timeframe[:-1])
    except ValueError:
        return float("nan")
    minutes = {"m": 1, "h": 60, "d": 1440, "w": 10080}.get(unit)
    if not minutes or size <= 0:
        return float("nan")
    return (365.0 * 1440.0) / (size * minutes)
