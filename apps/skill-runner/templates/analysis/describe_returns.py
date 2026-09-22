"""describe_returns — moments, tails and dependence of one return series.

```json
{
  "name": "describe_returns",
  "namespace": "analysis",
  "risk": "read",
  "description": "Moments with standard errors, normality and serial-dependence tests, tail index, max drawdown and jump share for one instrument's returns. Run it before any decision that assumes normality, independence or constant variance.",
  "input_schema": {
    "type": "object",
    "required": ["instrument", "timeframe", "start", "end", "seed"],
    "properties": {
      "instrument": {"type": "string"},
      "timeframe": {"type": "string", "enum": ["1m", "5m", "15m", "1h", "4h", "1d"]},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "seed": {"type": "integer"},
      "lags": {"type": "integer"}
    }
  },
  "output": {
    "summary_template": "{instrument} {timeframe}: n={n}, mean {mean} (se {mean_se}), sd {sd}, skew {skew}, excess kurtosis {kurtosis}. Normality {normality}; returns {serial_dependence}; variance {arch_verdict}. Tail index {hill_alpha}, max drawdown {max_dd_pct}, jump share {jump_share_pct}, recommended block length {block_length}.",
    "detail_fields": [
      ["instrument", ""], ["timeframe", ""], ["n", "bars"],
      ["mean", ""], ["mean_se", ""], ["sd", ""], ["skew", ""], ["kurtosis", ""],
      ["normality", ""], ["serial_dependence", ""], ["arch_verdict", ""],
      ["hill_alpha", ""], ["max_dd_pct", "%"], ["jump_share_pct", "%"],
      ["block_length", "bars"]
    ]
  },
  "evidence": [],
  "limits": {"wall_seconds": 60}
}
```

Every estimate carries its `n` and, where one is defined, its standard error
(AGENT-001 §10.2). A moment without an SE reads as though it were measured
exactly, and on a 2,000-bar window the standard error of excess kurtosis is
roughly 0.11 — large enough that "fat-tailed" and "not fat-tailed" are often the
same observation.

`block_length` is here because other tools need it. `family_wise_test`,
`trade_reshuffle_bootstrap` and `regime_failure_search` all resample in blocks,
and a block length hard-coded to a constant is a block length that is wrong for
every series but one. It is derived here from the first lag at which the
autocorrelation of |r| falls inside its own confidence band, which is the
dependence those bootstraps have to preserve.
"""

import json
import math
import sys

import numpy as np
import pandas as pd

try:
    from scipy import stats as _scipy_stats
except ImportError:  # pragma: no cover - the runner image ships scipy
    _scipy_stats = None


def provenance():
    """Library versions behind every number below."""
    out = {"python": sys.version.split()[0], "numpy": np.__version__, "pandas": pd.__version__}
    for name in ("scipy", "statsmodels"):
        try:
            out[name] = __import__(name).__version__
        except ImportError:
            out[name] = "absent"
    return out


def fail(code, message, fix):
    """Refuse in the catalogue's error shape (AGENT-006 §2.4a)."""
    print(json.dumps({"error": {"code": code, "message": message, "fix": fix, "retryable": False}}))
    raise SystemExit(0)


def chi2_sf(x, df):
    if _scipy_stats is None:
        fail(
            "statistics_unavailable",
            "scipy is not installed in the runner image, so no p-value can be computed.",
            "install scipy in the skill-runner image; a test without a p-value is not a test",
        )
    return float(_scipy_stats.chi2.sf(x, df))


def log_returns(df):
    close = pd.to_numeric(df["close"], errors="coerce").astype(float)
    r = np.diff(np.log(close.to_numpy()))
    return r[np.isfinite(r)]


def acf(x, nlags):
    x = x - x.mean()
    denom = float(np.dot(x, x))
    if denom == 0.0:
        return np.zeros(nlags + 1)
    out = [1.0]
    for k in range(1, nlags + 1):
        out.append(float(np.dot(x[k:], x[:-k])) / denom)
    return np.array(out)


def ljung_box(x, nlags):
    """Q = n(n+2) Σ ρ_k² / (n−k), χ²(nlags)."""
    n = len(x)
    rho = acf(x, nlags)[1:]
    q = n * (n + 2) * float(np.sum(rho**2 / (n - np.arange(1, nlags + 1))))
    return q, chi2_sf(q, nlags)


def arch_lm(r, nlags):
    """Engle's LM test: regress r_t² on its own lags, n·R² ~ χ²(nlags)."""
    e = r**2
    n = len(e) - nlags
    if n <= nlags + 1:
        return float("nan"), float("nan")
    y = e[nlags:]
    x = np.column_stack([np.ones(n)] + [e[nlags - k : -k] for k in range(1, nlags + 1)])
    beta, *_ = np.linalg.lstsq(x, y, rcond=None)
    resid = y - x @ beta
    ss_tot = float(np.sum((y - y.mean()) ** 2))
    r2 = 0.0 if ss_tot == 0 else 1.0 - float(np.sum(resid**2)) / ss_tot
    stat = n * r2
    return stat, chi2_sf(stat, nlags)


def hill_alpha(r):
    """Hill tail index on the upper 5% of |r|; higher alpha is a thinner tail."""
    a = np.sort(np.abs(r))[::-1]
    a = a[a > 0]
    k = max(10, int(math.ceil(0.05 * len(a))))
    if len(a) <= k + 1:
        return float("nan"), 0
    tail = a[:k]
    threshold = a[k]
    xi = float(np.mean(np.log(tail / threshold)))
    return (float("inf") if xi == 0 else 1.0 / xi), k


def max_drawdown(r):
    equity = np.exp(np.cumsum(r))
    peak = np.maximum.accumulate(equity)
    return float(np.min(equity / peak - 1.0))


def jump_share(r):
    """Share of total squared return contributed by |r| above 4 robust sigmas.

    The threshold is MAD-based rather than sd-based on purpose: the sd of a
    series with jumps is inflated *by* the jumps, so an sd threshold hides
    exactly what it is asked to find.
    """
    mad = float(np.median(np.abs(r - np.median(r))))
    sigma = 1.4826 * mad
    if sigma == 0:
        return 0.0
    mask = np.abs(r) > 4.0 * sigma
    total = float(np.sum(r**2))
    return 0.0 if total == 0 else float(np.sum(r[mask] ** 2)) / total


def block_length(r, max_lag):
    """First lag at which |r|'s autocorrelation is inside its ±1.96/√n band.

    This is the dependence a block bootstrap has to preserve; a constant would be
    right for one series and wrong for the rest.
    """
    n = len(r)
    band = 1.96 / math.sqrt(n)
    rho = acf(np.abs(r), max_lag)[1:]
    for k, v in enumerate(rho, start=1):
        if abs(v) < band:
            return k
    return max_lag


def main():
    payload = json.loads(sys.stdin.read())
    seed = int(payload["seed"])
    np.random.seed(seed)
    lags = int(payload.get("lags", 20))

    df = pd.read_parquet(payload["data_path"])
    r = log_returns(df)
    n = int(len(r))
    if n < 100:
        fail(
            "window_too_short",
            f"{n} returns is not enough for these tests; the smallest is Ljung-Box at {lags} lags.",
            "widen start/end, or use a finer timeframe",
        )
    if n <= 3 * lags:
        lags = max(5, n // 5)

    mean = float(np.mean(r))
    sd = float(np.std(r, ddof=1))
    skew = float(np.mean(((r - mean) / sd) ** 3))
    kurt = float(np.mean(((r - mean) / sd) ** 4)) - 3.0

    # Standard errors under the null of normality (Cramer 1946).
    mean_se = sd / math.sqrt(n)
    sd_se = sd / math.sqrt(2.0 * (n - 1))
    skew_se = math.sqrt(6.0 * n * (n - 1) / ((n - 2) * (n + 1) * (n + 3)))
    kurt_se = 2.0 * skew_se * math.sqrt((n * n - 1.0) / ((n - 3) * (n + 5)))

    jb = n / 6.0 * (skew**2 + kurt**2 / 4.0)
    jb_p = chi2_sf(jb, 2)
    lb_r, lb_r_p = ljung_box(r, lags)
    lb_r2, lb_r2_p = ljung_box(r**2, lags)
    arch_stat, arch_p = arch_lm(r, min(lags, 10))
    alpha, tail_k = hill_alpha(r)
    mdd = max_drawdown(r)
    jumps = jump_share(r)
    bl = block_length(r, min(lags * 2, max(10, n // 20)))

    detail = {
        "instrument": payload["instrument"],
        "timeframe": payload["timeframe"],
        "window": {"start": payload["start"], "end": payload["end"]},
        "n": n,
        "mean": round(mean, 8),
        "mean_se": round(mean_se, 8),
        "sd": round(sd, 8),
        "sd_se": round(sd_se, 8),
        "skew": round(skew, 4),
        "skew_se": round(skew_se, 4),
        "kurtosis": round(kurt, 4),
        "kurtosis_se": round(kurt_se, 4),
        "jarque_bera": {"stat": round(jb, 3), "p": round(jb_p, 6), "df": 2},
        "ljung_box": {
            "lags": lags,
            "returns": {"stat": round(lb_r, 3), "p": round(lb_r_p, 6)},
            "squared_returns": {"stat": round(lb_r2, 3), "p": round(lb_r2_p, 6)},
        },
        "arch_lm": {"lags": min(lags, 10), "stat": round(arch_stat, 3), "p": round(arch_p, 6)},
        "hill_alpha": round(alpha, 3),
        "hill_tail_observations": tail_k,
        "max_dd_pct": round(100.0 * mdd, 2),
        "jump_share_pct": round(100.0 * jumps, 2),
        "block_length": int(bl),
        "normality": "rejected" if jb_p < 0.05 else "not rejected",
        "serial_dependence": "autocorrelated" if lb_r_p < 0.05 else "no linear dependence",
        "arch_verdict": "clustered" if arch_p < 0.05 else "no ARCH effect",
        "provenance": provenance(),
        "seed": seed,
    }

    print(
        json.dumps(
            {
                "summary": None,  # rendered from the declared template by the harness
                "n": n,
                "se": round(mean_se, 8),
                "detail": detail,
            }
        )
    )


if __name__ == "__main__":
    main()
