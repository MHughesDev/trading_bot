"""stationarity — ADF and KPSS, which answer opposite questions on purpose.

```json
{
  "name": "stationarity",
  "namespace": "analysis",
  "risk": "read",
  "description": "Augmented Dickey-Fuller and KPSS tests on a price or return series, reported together with a four-way verdict. Run it before fitting anything that assumes a stable mean, and before treating a spread as mean-reverting.",
  "input_schema": {
    "type": "object",
    "required": ["instrument", "timeframe", "start", "end", "seed"],
    "properties": {
      "instrument": {"type": "string"},
      "timeframe": {"type": "string", "enum": ["1m", "5m", "15m", "1h", "4h", "1d"]},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "seed": {"type": "integer"},
      "series": {"type": "string", "enum": ["log_price", "returns"]},
      "regression": {"type": "string", "enum": ["c", "ct"]}
    }
  },
  "output": {
    "summary_template": "{instrument} {timeframe} {series} (n={n}): ADF p={adf_p} ({adf_verdict}), KPSS p={kpss_p} ({kpss_verdict}). Verdict: {verdict}. {caution}",
    "detail_fields": [
      ["instrument", ""], ["timeframe", ""], ["series", ""], ["n", "bars"],
      ["adf_p", ""], ["adf_verdict", ""], ["kpss_p", ""], ["kpss_verdict", ""],
      ["verdict", ""], ["caution", ""]
    ]
  },
  "evidence": [],
  "limits": {"wall_seconds": 60}
}
```

ADF's null is a unit root; KPSS's null is stationarity. Running one alone means
reading a failure to reject as a result, which it is not — a short window fails
to reject almost everything. Running both gives four outcomes, and two of them
say "the data cannot tell you", which is the answer that actually protects a
decision:

| ADF | KPSS | Verdict |
|---|---|---|
| rejects unit root | fails to reject stationarity | `stationary` |
| fails to reject | rejects | `unit_root` |
| rejects | rejects | `heteroskedastic_or_structural_break` |
| fails to reject | fails to reject | `underpowered` — not enough data to say |

KPSS's p-value is interpolated from the published critical-value table and is
reported as bounded (`>0.1`, `<0.01`) at the edges rather than extrapolated,
because an extrapolated p-value outside the table is a number with no
distribution behind it.
"""

import json
import math
import sys

import numpy as np
import pandas as pd


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


def adf(y, regression, maxlag=None):
    """Augmented Dickey-Fuller via statsmodels, which carries MacKinnon's surface.

    Deliberately not hand-rolled: the test statistic is easy and the *p-value* is
    not — it comes from MacKinnon's response surface, and a hand-rolled version
    would either hard-code a three-row critical-value table or invent a number.
    """
    try:
        from statsmodels.tsa.stattools import adfuller
    except ImportError:
        fail(
            "statistics_unavailable",
            "statsmodels is not installed, so the ADF p-value has no distribution behind it.",
            "install statsmodels in the skill-runner image",
        )
    stat, p, used_lag, nobs, crit, _ = adfuller(y, maxlag=maxlag, regression=regression, autolag="AIC")
    return float(stat), float(p), int(used_lag), int(nobs), {k: float(v) for k, v in crit.items()}


def kpss(y, regression):
    try:
        from statsmodels.tsa.stattools import kpss as _kpss
    except ImportError:
        fail(
            "statistics_unavailable",
            "statsmodels is not installed, so the KPSS p-value has no distribution behind it.",
            "install statsmodels in the skill-runner image",
        )
    import warnings

    with warnings.catch_warnings():
        # statsmodels warns when the statistic falls outside its table; that is
        # exactly the case we want to report as bounded rather than hide.
        warnings.simplefilter("ignore")
        stat, p, lags, crit = _kpss(y, regression=regression, nlags="auto")
    return float(stat), float(p), int(lags), {k: float(v) for k, v in crit.items()}


def main():
    payload = json.loads(sys.stdin.read())
    seed = int(payload["seed"])
    np.random.seed(seed)
    which = payload.get("series", "log_price")
    regression = payload.get("regression", "c")

    df = pd.read_parquet(payload["data_path"])
    close = pd.to_numeric(df["close"], errors="coerce").astype(float).to_numpy()
    close = close[np.isfinite(close) & (close > 0)]
    y = np.log(close) if which == "log_price" else np.diff(np.log(close))
    y = y[np.isfinite(y)]
    n = int(len(y))
    if n < 100:
        fail("window_too_short", f"{n} observations is not enough for a unit-root test.", "widen start/end")

    adf_stat, adf_p, adf_lag, adf_nobs, adf_crit = adf(y, regression)
    kpss_stat, kpss_p, kpss_lag, kpss_crit = kpss(y, regression)

    # statsmodels clips KPSS's p-value at the table edges; say so rather than
    # reporting the clipped number as if it were interpolated.
    kpss_bounded = None
    if kpss_p <= 0.01:
        kpss_bounded = "<0.01 (at the table edge)"
    elif kpss_p >= 0.10:
        kpss_bounded = ">0.10 (at the table edge)"

    adf_rejects = adf_p < 0.05
    kpss_rejects = kpss_p < 0.05
    if adf_rejects and not kpss_rejects:
        verdict, caution = "stationary", ""
    elif not adf_rejects and kpss_rejects:
        verdict, caution = "unit_root", "Differencing is required before any mean-reverting rule."
    elif adf_rejects and kpss_rejects:
        verdict = "heteroskedastic_or_structural_break"
        caution = "Both tests reject; suspect a regime change inside the window rather than a clean answer."
    else:
        verdict = "underpowered"
        caution = f"Neither test rejects at n={n}: the window cannot tell you. Do not read this as stationary."

    detail = {
        "instrument": payload["instrument"],
        "timeframe": payload["timeframe"],
        "window": {"start": payload["start"], "end": payload["end"]},
        "series": which,
        "regression": regression,
        "n": n,
        "adf": {
            "stat": round(adf_stat, 4),
            "p": round(adf_p, 5),
            "used_lag": adf_lag,
            "nobs": adf_nobs,
            "critical_values": {k: round(v, 4) for k, v in adf_crit.items()},
            "null": "a unit root is present",
        },
        "kpss": {
            "stat": round(kpss_stat, 4),
            "p": round(kpss_p, 5),
            "p_note": kpss_bounded,
            "lags": kpss_lag,
            "critical_values": {k: round(v, 4) for k, v in kpss_crit.items()},
            "null": "the series is stationary",
        },
        "adf_p": round(adf_p, 5),
        "kpss_p": round(kpss_p, 5),
        "adf_verdict": "rejects a unit root" if adf_rejects else "cannot reject a unit root",
        "kpss_verdict": "rejects stationarity" if kpss_rejects else "cannot reject stationarity",
        "verdict": verdict,
        "caution": caution,
        "provenance": provenance(),
        "seed": seed,
    }
    print(json.dumps({"summary": None, "n": n, "detail": detail}))


if __name__ == "__main__":
    main()
