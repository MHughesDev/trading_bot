"""How volatile is this, by every estimator that disagrees with the others.

```json
{
  "name": "estimate_volatility",
  "version": 1,
  "namespace": "analysis",
  "description": "Range and return based volatility estimates with GARCH and HAR forecasts, annualised, with today's level as a percentile of its own history. Use it to size a position or to decide whether a regime has changed.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "timeframe", "start", "end"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string", "description": "RFC 3339, inclusive"},
      "end": {"type": "string", "description": "RFC 3339, exclusive"},
      "window": {"type": "integer", "minimum": 10, "maximum": 2000, "default": 168,
                 "description": "bars in the trailing window used for the current level"}
    }
  },
  "summary_template": "annualised volatility {annualised_close_to_close} (close-to-close), {annualised_yang_zhang} (Yang-Zhang); currently at the {percentile} percentile of its own history; GARCH one-step {garch_forecast}, HAR {har_forecast}",
  "detail_fields": [
    ["annualised_close_to_close", ""],
    ["annualised_yang_zhang", ""],
    ["percentile", "pct"],
    ["garch_forecast", ""],
    ["har_forecast", ""]
  ],
  "wall_seconds": 120,
  "series": [{"role": "primary"}]
}
```

Why five estimators and not one
-------------------------------
They disagree, and *how* they disagree is the information.

* **Close-to-close** uses one price per bar and throws away the other three. It
  is the only one that is right when the intrabar path is not trustworthy, which
  on a thin instrument is most of the time.
* **Parkinson** uses the range. It is roughly five times more efficient than
  close-to-close and assumes no drift and continuous observation; on a real book
  it is biased *down*, because the true high and low happen between prints.
* **Garman-Klass** adds the open and close; **Rogers-Satchell** drops the
  no-drift assumption; **Yang-Zhang** handles overnight gaps, which for crypto
  means nothing and for equities means most of the variance.
* **Bipower variation** is realised variance with the jumps taken out.

A Parkinson estimate far below close-to-close means the bars' highs and lows are
not real — stale quotes, or an aggregation that took the last print rather than
the extremes. That is a data-quality finding and it is reported as one, because
it invalidates any strategy whose entries are stops.

The forecasts
-------------
GARCH(1,1) by maximum likelihood, and HAR on log realised variance. Both are
reported with the unconditional level beside them: a GARCH forecast that has
collapsed to the unconditional mean is a GARCH forecast that is saying nothing,
and it looks identical to one that is confident.
"""

_TRADING_MINUTE = 1.0


def _range_estimators(frame):
    """Parkinson, Garman-Klass, Rogers-Satchell and Yang-Zhang, per bar."""
    o = pd.to_numeric(frame["open"], errors="coerce").to_numpy(dtype=float)
    h = pd.to_numeric(frame["high"], errors="coerce").to_numpy(dtype=float)
    l = pd.to_numeric(frame["low"], errors="coerce").to_numpy(dtype=float)
    c = pd.to_numeric(frame["close"], errors="coerce").to_numpy(dtype=float)

    ok = np.isfinite(o) & np.isfinite(h) & np.isfinite(l) & np.isfinite(c)
    ok &= (o > 0) & (h > 0) & (l > 0) & (c > 0) & (h >= l)
    o, h, l, c = o[ok], h[ok], l[ok], c[ok]
    n = o.size
    if n < 2:
        return None

    hl = np.log(h / l)
    co = np.log(c / o)
    ho = np.log(h / o)
    lo = np.log(l / o)

    parkinson = float(np.mean(hl**2) / (4.0 * math.log(2.0)))
    garman_klass = float(np.mean(0.5 * hl**2 - (2.0 * math.log(2.0) - 1.0) * co**2))
    rogers_satchell = float(np.mean(ho * (ho - co) + lo * (lo - co)))

    # Yang-Zhang: overnight + open-to-close + Rogers-Satchell, weighted by the
    # k that minimises variance. For a continuously traded instrument the
    # overnight term is near zero and this collapses towards Rogers-Satchell,
    # which is the correct behaviour rather than a degenerate one.
    oc = np.log(o[1:] / c[:-1])
    sigma_o = float(np.var(oc, ddof=1)) if oc.size > 1 else 0.0
    sigma_c = float(np.var(co, ddof=1)) if n > 1 else 0.0
    k = 0.34 / (1.34 + (n + 1.0) / (n - 1.0))
    yang_zhang = sigma_o + k * sigma_c + (1.0 - k) * rogers_satchell

    return {
        "n": n,
        "parkinson": parkinson,
        "garman_klass": garman_klass,
        "rogers_satchell": rogers_satchell,
        "yang_zhang": max(yang_zhang, 0.0),
    }


def _garch11(r):
    """GARCH(1,1) by maximum likelihood, returning the one-step variance.

    Written out rather than imported: see apps/skill-runner/requirements.txt for
    why statsmodels and arch are not in the runner's stack. The likelihood is the
    standard Gaussian quasi-likelihood, which is consistent for the variance
    parameters even when the innovations are fat-tailed — which they always are
    here, so this is the estimator being used correctly rather than a shortcut.
    """
    if not _HAVE_SCIPY or r.size < 100:
        return None
    x = r - r.mean()
    var = float(np.var(x, ddof=1))
    if var <= 0:
        return None

    def negative_log_likelihood(theta):
        omega, alpha, beta = np.exp(theta)
        if alpha + beta >= 0.999:
            return 1e10
        h = np.empty(x.size)
        h[0] = var
        for t in range(1, x.size):
            h[t] = omega + alpha * x[t - 1] ** 2 + beta * h[t - 1]
            if not np.isfinite(h[t]) or h[t] <= 0:
                return 1e10
        return 0.5 * float(np.sum(np.log(h) + x**2 / h))

    start = np.log(np.array([var * 0.05, 0.08, 0.88]))
    try:
        fit = _opt.minimize(
            negative_log_likelihood,
            start,
            method="Nelder-Mead",
            options={"maxiter": 2000, "xatol": 1e-8, "fatol": 1e-8},
        )
    except Exception:  # noqa: BLE001 - a failed fit is a result, not a crash
        return None
    if not fit.success and fit.fun >= 1e10:
        return None

    omega, alpha, beta = np.exp(fit.x)
    h = np.empty(x.size)
    h[0] = var
    for t in range(1, x.size):
        h[t] = omega + alpha * x[t - 1] ** 2 + beta * h[t - 1]
    persistence = float(alpha + beta)
    return {
        "omega": float(omega),
        "alpha": float(alpha),
        "beta": float(beta),
        "persistence": persistence,
        "unconditional_variance": float(omega / (1.0 - persistence)) if persistence < 1 else None,
        "forecast_variance": float(omega + alpha * x[-1] ** 2 + beta * h[-1]),
        "converged": bool(fit.success),
    }


def _har(r, lags=(1, 5, 22)):
    """HAR: log realised variance on its own daily, weekly and monthly averages.

    On log RV rather than RV, because RV is right-skewed by construction and an
    OLS fit on it is dominated by a handful of crisis bars.
    """
    rv = r**2
    floor = np.percentile(rv[rv > 0], 1) if np.any(rv > 0) else 1e-12
    y_all = np.log(np.maximum(rv, floor))
    longest = max(lags)
    if y_all.size < longest * 3:
        return None

    averages = []
    for lag in lags:
        cumulative = np.convolve(y_all, np.ones(lag) / lag, mode="valid")
        averages.append(cumulative[longest - lag : -1])
    target = y_all[longest:]
    width = min(len(target), min(len(a) for a in averages))
    x = np.column_stack([np.ones(width)] + [a[-width:] for a in averages])
    y = target[-width:]
    coef, *_ = np.linalg.lstsq(x, y, rcond=None)

    latest = [float(np.mean(y_all[-lag:])) for lag in lags]
    log_forecast = float(coef[0] + sum(c * v for c, v in zip(coef[1:], latest)))
    resid = y - x @ coef
    # Exp of a fitted log is biased low by half the residual variance; the
    # correction is the standard one and is applied rather than left to whoever
    # reads the number.
    sigma2 = float(np.var(resid, ddof=len(coef))) if width > len(coef) else 0.0
    return {
        "coefficients": {f"lag_{lag}": float(c) for lag, c in zip(lags, coef[1:])},
        "intercept": float(coef[0]),
        "forecast_variance": float(math.exp(log_forecast + 0.5 * sigma2)),
        "n": int(width),
    }


def _ewma(r, lam=0.94):
    """RiskMetrics EWMA. One parameter, no fit, and a useful sanity check on GARCH."""
    v = float(np.var(r, ddof=1))
    for x in r:
        v = lam * v + (1.0 - lam) * float(x) ** 2
    return v


def run(payload: dict) -> dict:
    frame = load(payload, "primary")
    r = close_returns(frame)
    n = int(r.size)
    if n < 60:
        return refuse(
            "volatility estimates below 60 bars are dominated by their own standard error",
            n_bars=n,
        )

    timeframe = payload.get("timeframe") or "1h"
    per_year = bars_per_year(timeframe)
    if not math.isfinite(per_year):
        return refuse(f"cannot annualise an unrecognised timeframe {timeframe!r}")

    def annualise(variance_per_bar):
        if variance_per_bar is None or variance_per_bar < 0:
            return None
        return math.sqrt(variance_per_bar * per_year)

    close_to_close = float(np.var(r, ddof=1))
    ranges = _range_estimators(frame)

    # Realised and bipower, over the whole window, per bar.
    rv = float(np.mean(r**2))
    mu1 = math.sqrt(2.0 / math.pi)
    bv = float(np.mean(np.abs(r[1:]) * np.abs(r[:-1])) / (mu1**2))
    # Realised quarticity: what the standard error of RV is built from.
    rq = float(n / 3.0 * np.mean(r**4))

    window = int(payload.get("window") or 168)
    window = max(10, min(window, n))
    recent = float(np.var(r[-window:], ddof=1))

    # Where the current level sits in its own history, by nearest rank over
    # rolling windows. A percentile of an absolute number means nothing across
    # instruments; a percentile of its own history means something on all of them.
    rolling = np.array(
        [float(np.var(r[i : i + window], ddof=1)) for i in range(0, n - window + 1)]
    )
    percentile = (
        float(100.0 * np.mean(rolling <= recent)) if rolling.size > 1 else None
    )

    garch = _garch11(r)
    har = _har(r)
    ewma = _ewma(r)

    warnings = []
    if ranges:
        # The data-quality finding: a range estimator far below close-to-close
        # means the bars' extremes are not real.
        ratio = ranges["parkinson"] / close_to_close if close_to_close > 0 else None
        if ratio is not None and ratio < 0.5:
            warnings.append(
                f"the Parkinson estimate is {ratio:.0%} of close-to-close. The high and low in "
                f"these bars are probably not the true intrabar extremes — stale quotes, or an "
                f"aggregation that recorded the last print rather than the range. Any strategy "
                f"whose entries or stops depend on touching a level is untestable on this data."
            )
    if garch and garch["persistence"] > 0.995:
        warnings.append(
            f"GARCH persistence is {garch['persistence']:.4f}, effectively a unit root in "
            f"variance: the model is close to IGARCH, its unconditional level is not defined, "
            f"and forecasts beyond one step should not be used."
        )
    if garch is None:
        warnings.append(
            "the GARCH fit did not converge; the EWMA and HAR forecasts are the usable ones."
        )

    detail = {
        "n_bars": n,
        "annualisation": {"bars_per_year": per_year, "timeframe": timeframe},
        "variance_per_bar": {
            "close_to_close": close_to_close,
            "realised": rv,
            "bipower": bv,
            "realised_quarticity": rq,
            "recent_window": recent,
            "ewma": ewma,
            **({k: v for k, v in ranges.items() if k != "n"} if ranges else {}),
        },
        "annualised_close_to_close": annualise(close_to_close),
        "annualised_yang_zhang": annualise(ranges["yang_zhang"]) if ranges else None,
        "annualised_parkinson": annualise(ranges["parkinson"]) if ranges else None,
        "annualised_recent": annualise(recent),
        "percentile": percentile,
        "window_bars": window,
        "garch": garch,
        "garch_forecast": annualise(garch["forecast_variance"]) if garch else None,
        "har": har,
        "har_forecast": annualise(har["forecast_variance"]) if har else None,
        "ewma_forecast": annualise(ewma),
        "jump_fraction": max(0.0, (rv - bv) / rv) if rv > 0 else None,
        "warnings": warnings,
        "window": {
            "instrument_id": payload.get("instrument_id"),
            "timeframe": timeframe,
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }

    # The standard error of realised variance, from the quarticity. Reported as
    # the error on the headline because it is the one estimate here whose error
    # has a closed form worth trusting.
    se_rv = math.sqrt(2.0 * rq / n) if rq > 0 else None
    return result(detail, n=n, se=se_rv)
