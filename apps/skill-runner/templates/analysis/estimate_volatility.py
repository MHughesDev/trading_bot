"""estimate_volatility — range and realised volatility estimators, and forecasts.

```json
{
  "name": "estimate_volatility",
  "namespace": "analysis",
  "risk": "read",
  "description": "Annualised volatility by range estimator (Parkinson, Garman-Klass, Rogers-Satchell, Yang-Zhang) and realised measures (RV, bipower, quarticity), with EWMA, GARCH(1,1) and HAR forecasts and where today's level sits in its own year. Use it to size positions and to tell a quiet regime from a loud one.",
  "input_schema": {
    "type": "object",
    "required": ["instrument", "timeframe", "start", "end", "seed"],
    "properties": {
      "instrument": {"type": "string"},
      "timeframe": {"type": "string", "enum": ["1m", "5m", "15m", "1h", "4h", "1d"]},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "seed": {"type": "integer"},
      "estimators": {"type": "array", "items": {"type": "string", "enum": ["pk", "gk", "rs", "yz", "rv", "bv", "rq"]}},
      "forecast": {"type": "array", "items": {"type": "string", "enum": ["ewma", "garch", "har"]}}
    }
  },
  "output": {
    "summary_template": "{instrument} {timeframe}: Yang-Zhang {yz_annual_pct} annualised over n={n} bars (se {yz_se_pct}), realised {rv_annual_pct}, bipower {bv_annual_pct}. Jump component {jump_component_pct}. Now at the {percentile_1y} of its trailing year. Forecasts: {forecast_summary}.",
    "detail_fields": [
      ["instrument", ""], ["timeframe", ""], ["n", "bars"],
      ["yz_annual_pct", "%"], ["yz_se_pct", "%"], ["rv_annual_pct", "%"],
      ["bv_annual_pct", "%"], ["jump_component_pct", "%"],
      ["percentile_1y", "pctile"], ["forecast_summary", ""]
    ]
  },
  "evidence": [],
  "limits": {"wall_seconds": 90}
}
```

Four range estimators rather than one because they fail in different places and
the disagreement is the information. Parkinson ignores the open-to-close drift;
Garman-Klass assumes no overnight gap; Rogers-Satchell tolerates drift but not
gaps; Yang-Zhang handles both and is the one the summary leads with. When they
agree the number is solid; when Yang-Zhang and Parkinson diverge, the series has
gaps or drift that a single-estimator answer would have hidden.

Bipower variation is reported beside realised variance for the same reason: RV
counts jumps as volatility, BV does not, and `jump_component_pct` — the gap
between them — is what separates a genuinely volatile market from a calm one
with three news events in it. Sizing off RV in the second case over-hedges all
month to pay for three days.
"""

import json
import math
import sys

import numpy as np
import pandas as pd

BARS_PER_YEAR = {
    "1m": 525_600,
    "5m": 105_120,
    "15m": 35_040,
    "1h": 8_760,
    "4h": 2_190,
    "1d": 365,
}


def provenance():
    out = {"python": sys.version.split()[0], "numpy": np.__version__, "pandas": pd.__version__}
    for name in ("scipy", "statsmodels"):
        try:
            out[name] = __import__(name).__version__
        except ImportError:
            out[name] = "absent"
    return out


def fail(code, message, fix):
    print(json.dumps({"error": {"code": code, "message": message, "fix": fix, "retryable": False}}))
    raise SystemExit(0)


def parkinson(h, l):
    return math.sqrt(float(np.mean((np.log(h / l) ** 2))) / (4.0 * math.log(2.0)))


def garman_klass(o, h, l, c):
    a = 0.5 * np.log(h / l) ** 2
    b = (2.0 * math.log(2.0) - 1.0) * np.log(c / o) ** 2
    return math.sqrt(max(float(np.mean(a - b)), 0.0))


def rogers_satchell(o, h, l, c):
    v = np.log(h / c) * np.log(h / o) + np.log(l / c) * np.log(l / o)
    return math.sqrt(max(float(np.mean(v)), 0.0))


def yang_zhang(o, h, l, c):
    """Overnight + open-to-close + Rogers-Satchell, with the minimum-variance k."""
    n = len(o)
    if n < 3:
        return float("nan")
    prev_c = c[:-1]
    o1, h1, l1, c1 = o[1:], h[1:], l[1:], c[1:]
    overnight = np.log(o1 / prev_c)
    openclose = np.log(c1 / o1)
    v_o = float(np.var(overnight, ddof=1))
    v_c = float(np.var(openclose, ddof=1))
    v_rs = max(
        float(np.mean(np.log(h1 / c1) * np.log(h1 / o1) + np.log(l1 / c1) * np.log(l1 / o1))),
        0.0,
    )
    k = 0.34 / (1.34 + (n + 1.0) / (n - 1.0))
    return math.sqrt(max(v_o + k * v_c + (1.0 - k) * v_rs, 0.0))


def realised(r):
    return math.sqrt(float(np.sum(r**2)) / len(r))


def bipower(r):
    """Barndorff-Nielsen/Shephard BV: jump-robust, so RV − BV is the jump part."""
    mu1 = math.sqrt(2.0 / math.pi)
    a = np.abs(r[1:]) * np.abs(r[:-1])
    return math.sqrt(max(float(np.sum(a)) / (mu1**2 * len(a)), 0.0))


def quarticity(r):
    n = len(r)
    return float(n / 3.0 * np.sum(r**4))


def ewma_forecast(r, lam=0.94):
    var = float(np.var(r, ddof=1))
    for x in r:
        var = lam * var + (1.0 - lam) * float(x) ** 2
    return math.sqrt(var)


def garch11(r, iters=200):
    """GARCH(1,1) by grid-refined Gaussian quasi-likelihood.

    A hand-rolled optimiser rather than a package because the runner image may
    not carry `arch`, and a forecast that silently disappears when a dependency
    is missing is worse than a coarse one that is always there. The grid is
    deterministic, so two runs with the same seed give the same parameters.
    """
    r = r - r.mean()
    s2 = float(np.var(r, ddof=1))
    best = None
    for alpha in np.linspace(0.02, 0.25, 12):
        for beta in np.linspace(0.60, 0.97, 16):
            if alpha + beta >= 0.999:
                continue
            omega = s2 * (1.0 - alpha - beta)
            h = np.empty(len(r))
            h[0] = s2
            for t in range(1, len(r)):
                h[t] = omega + alpha * r[t - 1] ** 2 + beta * h[t - 1]
            ll = -0.5 * float(np.sum(np.log(h) + r**2 / h))
            if best is None or ll > best[0]:
                best = (ll, float(alpha), float(beta), float(omega), h)
    if best is None:
        return float("nan"), {}
    _, alpha, beta, omega, h = best
    next_h = omega + alpha * float(r[-1]) ** 2 + beta * float(h[-1])
    return math.sqrt(next_h), {"alpha": round(alpha, 4), "beta": round(beta, 4), "persistence": round(alpha + beta, 4)}


def har_forecast(r, short=1, mid=5, long=22):
    """HAR: today's variance on yesterday's, last week's and last month's."""
    v = r**2
    n = len(v)
    if n < long + 2:
        return float("nan")
    rows, ys = [], []
    for t in range(long, n):
        rows.append([1.0, v[t - short : t].mean(), v[t - mid : t].mean(), v[t - long : t].mean()])
        ys.append(v[t])
    x = np.array(rows)
    y = np.array(ys)
    beta, *_ = np.linalg.lstsq(x, y, rcond=None)
    last = np.array([1.0, v[-short:].mean(), v[-mid:].mean(), v[-long:].mean()])
    return math.sqrt(max(float(last @ beta), 0.0))


def main():
    payload = json.loads(sys.stdin.read())
    seed = int(payload["seed"])
    np.random.seed(seed)
    timeframe = payload["timeframe"]
    wanted = payload.get("estimators", ["pk", "gk", "rs", "yz", "rv", "bv", "rq"])
    forecasts_wanted = payload.get("forecast", ["ewma", "garch", "har"])

    df = pd.read_parquet(payload["data_path"])
    for col in ("open", "high", "low", "close"):
        if col not in df.columns:
            fail(
                "missing_column",
                f"the materialised bars have no `{col}`; range estimators need OHLC.",
                "request an OHLCV series, not a close-only one",
            )
    o = pd.to_numeric(df["open"], errors="coerce").astype(float).to_numpy()
    h = pd.to_numeric(df["high"], errors="coerce").astype(float).to_numpy()
    l = pd.to_numeric(df["low"], errors="coerce").astype(float).to_numpy()
    c = pd.to_numeric(df["close"], errors="coerce").astype(float).to_numpy()
    ok = np.isfinite(o) & np.isfinite(h) & np.isfinite(l) & np.isfinite(c) & (l > 0)
    o, h, l, c = o[ok], h[ok], l[ok], c[ok]
    n = int(len(c))
    if n < 60:
        fail("window_too_short", f"{n} bars is not enough to estimate volatility.", "widen start/end")

    r = np.diff(np.log(c))
    ann = math.sqrt(BARS_PER_YEAR.get(timeframe, 365))

    per_bar = {}
    if "pk" in wanted:
        per_bar["pk"] = parkinson(h, l)
    if "gk" in wanted:
        per_bar["gk"] = garman_klass(o, h, l, c)
    if "rs" in wanted:
        per_bar["rs"] = rogers_satchell(o, h, l, c)
    if "yz" in wanted:
        per_bar["yz"] = yang_zhang(o, h, l, c)
    if "rv" in wanted:
        per_bar["rv"] = realised(r)
    if "bv" in wanted:
        per_bar["bv"] = bipower(r)

    # SE of a volatility estimate under normality: sigma / sqrt(2n).
    estimators = {
        k: {
            "annual_pct": round(100.0 * v * ann, 3),
            "se_pct": round(100.0 * v * ann / math.sqrt(2.0 * n), 3),
        }
        for k, v in per_bar.items()
        if np.isfinite(v)
    }

    rv = per_bar.get("rv", realised(r))
    bv = per_bar.get("bv", bipower(r))
    jump_component = max(0.0, (rv**2 - bv**2) / rv**2) if rv > 0 else 0.0

    fc = {}
    if "ewma" in forecasts_wanted:
        fc["ewma"] = round(100.0 * ewma_forecast(r) * ann, 3)
    if "garch" in forecasts_wanted:
        g, params = garch11(r)
        fc["garch"] = round(100.0 * g * ann, 3)
        fc["garch_params"] = params
    if "har" in forecasts_wanted:
        hf = har_forecast(r)
        fc["har"] = None if not np.isfinite(hf) else round(100.0 * hf * ann, 3)

    # Where the current level sits in its own trailing year of the same estimator.
    per_year = BARS_PER_YEAR.get(timeframe, 365)
    win = max(20, min(len(r) // 4, per_year // 12 or 20))
    rolling = pd.Series(r).rolling(win).std(ddof=1).dropna().to_numpy()
    percentile = (
        int(round(100.0 * float((rolling <= rolling[-1]).mean()))) if len(rolling) > 1 else 50
    )

    fc_summary = ", ".join(
        f"{k} {v}%" for k, v in fc.items() if k != "garch_params" and v is not None
    )

    detail = {
        "instrument": payload["instrument"],
        "timeframe": timeframe,
        "window": {"start": payload["start"], "end": payload["end"]},
        "n": n,
        "estimators": estimators,
        "yz_annual_pct": estimators.get("yz", {}).get("annual_pct"),
        "yz_se_pct": estimators.get("yz", {}).get("se_pct"),
        "rv_annual_pct": estimators.get("rv", {}).get("annual_pct"),
        "bv_annual_pct": estimators.get("bv", {}).get("annual_pct"),
        "jump_component_pct": round(100.0 * jump_component, 2),
        "quarticity": round(quarticity(r), 12) if "rq" in wanted else None,
        "forecast": fc,
        "forecast_summary": fc_summary or "none requested",
        "percentile_1y": percentile,
        "rolling_window_bars": int(win),
        "provenance": provenance(),
        "seed": seed,
    }
    print(json.dumps({"summary": None, "n": n, "detail": detail}))


if __name__ == "__main__":
    main()
