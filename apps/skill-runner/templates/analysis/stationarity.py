"""Is this series mean-reverting, or is it a random walk wearing a trend.

```json
{
  "name": "stationarity",
  "version": 1,
  "namespace": "analysis",
  "description": "Augmented Dickey-Fuller and KPSS on the level and on returns, with the OU half-life when the level is mean-reverting. Run it before building anything that assumes a price reverts to a mean.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "timeframe", "start", "end"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "transform": {"type": "string", "enum": ["log_price", "price"], "default": "log_price"}
    }
  },
  "summary_template": "ADF {adf_statistic} ({adf_verdict} at 5%), KPSS {kpss_statistic} ({kpss_verdict} at 5%) - together they say the level {joint_verdict}; half-life {half_life} bars",
  "detail_fields": [
    ["adf_statistic", ""],
    ["adf_verdict", ""],
    ["kpss_statistic", ""],
    ["kpss_verdict", ""],
    ["joint_verdict", ""],
    ["fracdiff_min_d", ""],
    ["half_life", "bars"]
  ],
  "wall_seconds": 120,
  "series": [{"role": "primary"}]
}
```

Why both tests, and why the joint verdict is the answer
-------------------------------------------------------
They have opposite null hypotheses, and that is the point:

* **ADF** nulls a unit root. Failing to reject is *not* evidence of a unit root —
  it is the absence of evidence against one, and ADF is famously low-powered
  against a near-unit root. Reading "ADF did not reject" as "the series is a
  random walk" is the standard mistake.
* **KPSS** nulls stationarity. Failing to reject is not evidence of stationarity
  either.

Four combinations, and two of them are honest "do not know":

| ADF rejects | KPSS rejects | Conclusion |
|---|---|---|
| yes | no | stationary — the only clean one |
| no | yes | a unit root — the other clean one |
| no | no | the sample is too short or too noisy to tell |
| yes | yes | neither model fits; usually a structural break or fractional integration |

A tool that reported only ADF would turn the third row into "random walk" and the
fourth into "mean-reverting", both confidently. So the joint verdict is what the
summary carries.

Critical values
---------------
MacKinnon's response-surface constants for ADF (constant, no trend) and the
standard Kwiatkowski-Phillips-Schmidt-Shin table for KPSS, both embedded — see
`apps/skill-runner/requirements.txt` for why this file does not import
statsmodels. Critical values are reported alongside the statistic rather than
collapsed into a p-value, because an interpolated p-value from a table invites a
precision the table does not have.
"""

# MacKinnon (1991, 2010), constant and no trend, one variable.
# tau(T) = tau_inf + tau1/T + tau2/T^2.
_ADF_CRITICAL = {
    "1%": (-3.43035, -6.5393, -16.786),
    "5%": (-2.86154, -2.8903, -4.234),
    "10%": (-2.56677, -1.5384, -2.809),
}

# Kwiatkowski, Phillips, Schmidt and Shin (1992), Table 1, level stationarity.
_KPSS_CRITICAL = {"10%": 0.347, "5%": 0.463, "2.5%": 0.574, "1%": 0.739}


def _adf_critical(n):
    return {
        level: c0 + c1 / n + c2 / (n * n)
        for level, (c0, c1, c2) in _ADF_CRITICAL.items()
    }


def _select_lags(y, max_lag):
    """Lag order by BIC over the ADF regression itself.

    BIC rather than AIC: AIC over-selects on long series, and every extra lag
    costs power in exactly the test that has least to spare.
    """
    best, best_ic = 0, math.inf
    for lag in range(0, max_lag + 1):
        fit = _adf_regression(y, lag)
        if fit is None:
            continue
        n, ssr, k = fit["n"], fit["ssr"], fit["k"]
        if ssr <= 0:
            continue
        ic = n * math.log(ssr / n) + k * math.log(n)
        if ic < best_ic:
            best, best_ic = lag, ic
    return best


def _adf_regression(y, lag):
    """dy_t = a + rho*y_{t-1} + sum b_i dy_{t-i} + e."""
    dy = np.diff(y)
    n_total = dy.size
    if n_total <= lag + 3:
        return None
    target = dy[lag:]
    columns = [np.ones(target.size), y[lag:-1]]
    for i in range(1, lag + 1):
        columns.append(dy[lag - i : n_total - i])
    x = np.column_stack(columns)
    if x.shape[0] <= x.shape[1] + 1:
        return None
    coef, *_ = np.linalg.lstsq(x, target, rcond=None)
    resid = target - x @ coef
    ssr = float(resid @ resid)
    dof = x.shape[0] - x.shape[1]
    if dof <= 0:
        return None
    sigma2 = ssr / dof
    try:
        covariance = sigma2 * np.linalg.inv(x.T @ x)
    except np.linalg.LinAlgError:
        return None
    se_rho = math.sqrt(max(covariance[1, 1], 0.0))
    return {
        "rho": float(coef[1]),
        "se_rho": se_rho,
        "statistic": float(coef[1] / se_rho) if se_rho > 0 else float("nan"),
        "n": int(x.shape[0]),
        "k": int(x.shape[1]),
        "ssr": ssr,
    }


def _kpss(y, lags=None):
    """KPSS with a Newey-West long-run variance.

    The bandwidth is Schwert's rule. It matters: KPSS with too few lags rejects
    stationarity on any series with mild autocorrelation, which is all of them.
    """
    n = y.size
    if n < 30:
        return None
    resid = y - y.mean()
    partial = np.cumsum(resid)
    if lags is None:
        lags = int(math.floor(4.0 * (n / 100.0) ** 0.25))
    lags = max(1, min(lags, n - 2))

    s2 = float(resid @ resid) / n
    for lag in range(1, lags + 1):
        weight = 1.0 - lag / (lags + 1.0)
        s2 += 2.0 * weight * float(resid[lag:] @ resid[:-lag]) / n
    if s2 <= 0:
        return None
    statistic = float((partial @ partial) / (n * n * s2))
    return {"statistic": statistic, "lags": lags, "long_run_variance": s2}


def _half_life(y):
    """OU half-life from an AR(1) fit on the level.

    `None` when the fitted coefficient is not mean-reverting, rather than a
    negative or infinite half-life: "reverts in -4 bars" is a number that gets
    quoted.
    """
    dy = np.diff(y)
    x = np.column_stack([np.ones(dy.size), y[:-1]])
    coef, *_ = np.linalg.lstsq(x, dy, rcond=None)
    theta = float(coef[1])
    if theta >= 0:
        return None
    return float(-math.log(2.0) / math.log(1.0 + theta)) if (1.0 + theta) > 0 else None


_FRACDIFF_WINDOW = 64
_FRACDIFF_GRID = (0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9)


def _fracdiff_weights(d, n):
    """The binomial series of (1 - B)^d: w0 = 1, wk = -w(k-1) * (d - k + 1) / k."""
    w = [1.0]
    for k in range(1, n):
        w.append(-w[-1] * (d - k + 1.0) / k)
    return np.asarray(w, dtype=float)


def _fracdiff_last(log_price, d, n=_FRACDIFF_WINDOW):
    """Fractional difference of order `d` at the last observation, over `n` bars.

    The value the feature engine serves as `fracdiff_N_D` (D in hundredths): a
    fixed window of the log close, newest bar weighted 1. A fixed window rather
    than an expanding one, so the value is a function of exactly its declared
    lookback. Compared with the engine over the same bars by
    `features_f2_reconciliation`.
    """
    x = np.asarray(log_price, dtype=float)
    if x.size < n:
        return None
    window = x[-n:][::-1]  # newest first
    return float(np.dot(_fracdiff_weights(d, n), window))


def _fracdiff_series(log_price, d, n=_FRACDIFF_WINDOW):
    """`_fracdiff_last` at every bar that has a full window."""
    x = np.asarray(log_price, dtype=float)
    if x.size < n:
        return np.empty(0, dtype=float)
    w = _fracdiff_weights(d, n)
    # correlate(x, w[::-1]) puts w[0] on the newest bar of each window.
    return np.correlate(x, w[::-1], mode="valid")


def _fracdiff_min_d(level, critical):
    """The smallest order on the grid at which ADF rejects a unit root.

    Differencing buys stationarity at the cost of memory, and an integer
    difference spends all of it. The smallest order that is stationary is the
    one that keeps the most, which is the number a feature pipeline wants and
    the one ADF and KPSS on the level cannot give. `None` when no order on the
    grid rejects, which is a statement about this window and not a failure.
    """
    for d in _FRACDIFF_GRID:
        series = _fracdiff_series(level, d)
        if series.size < 60:
            return None
        fit = _adf_regression(series, _select_lags(series, 4))
        if fit and math.isfinite(fit["statistic"]):
            if fit["statistic"] < _adf_critical(series.size)["5%"]:
                return d
    return None


def run(payload: dict) -> dict:
    frame = load(payload, "primary")
    close = pd.to_numeric(frame["close"], errors="coerce").to_numpy(dtype=float)
    close = close[np.isfinite(close) & (close > 0)]
    n = int(close.size)
    if n < 100:
        return refuse(
            "a unit-root test on fewer than 100 observations has almost no power; it will fail "
            "to reject whatever is true",
            n_bars=n,
        )

    transform = payload.get("transform") or "log_price"
    level = np.log(close) if transform == "log_price" else close
    returns = np.diff(level)

    max_lag = int(math.floor(12.0 * (n / 100.0) ** 0.25))
    lag = _select_lags(level, min(max_lag, max(1, n // 10)))
    adf_level = _adf_regression(level, lag)
    adf_returns = _adf_regression(returns, max(0, lag - 1))
    critical = _adf_critical(n)

    kpss_level = _kpss(level)
    kpss_returns = _kpss(returns)

    adf_stat = adf_level["statistic"] if adf_level else float("nan")
    adf_rejects = math.isfinite(adf_stat) and adf_stat < critical["5%"]
    kpss_stat = kpss_level["statistic"] if kpss_level else float("nan")
    kpss_rejects = math.isfinite(kpss_stat) and kpss_stat > _KPSS_CRITICAL["5%"]

    if adf_rejects and not kpss_rejects:
        joint = "is stationary"
    elif kpss_rejects and not adf_rejects:
        joint = "has a unit root"
    elif not adf_rejects and not kpss_rejects:
        joint = "cannot be classified from this sample — neither test rejects, which is a "
        joint += "statement about the sample's length, not about the series"
    else:
        joint = "fits neither model — look for a structural break or fractional integration"

    half_life = _half_life(level) if adf_rejects else None
    fracdiff_min_d = _fracdiff_min_d(np.log(close), critical)

    warnings = []
    if not adf_rejects and not kpss_rejects:
        warnings.append(
            "both tests failed to reject their own nulls, which are opposites. That is an "
            "indeterminate result and it should not be reported as either a random walk or a "
            "mean-reverting series."
        )
    if adf_rejects and kpss_rejects:
        warnings.append(
            "both tests rejected, which no single model explains. The usual causes are a "
            "structural break inside the window and fractional integration; a mean-reversion "
            "strategy fitted here will be fitted to the break."
        )
    if adf_returns and adf_returns["statistic"] > critical["5%"]:
        warnings.append(
            "the *returns* do not reject a unit root either, which almost always means the "
            "price column is wrong — a cumulative series, or an index that was already "
            "differenced."
        )
    if half_life is not None and half_life > n / 4:
        warnings.append(
            f"the half-life of {half_life:.0f} bars is more than a quarter of the "
            f"{n}-bar sample. A reversion slower than the window cannot be distinguished from "
            f"no reversion at all."
        )

    detail = {
        "n_bars": n,
        "transform": transform,
        "adf_statistic": adf_stat,
        "adf_lags": lag,
        "adf_critical_values": critical,
        "adf_verdict": "rejects a unit root" if adf_rejects else "does not reject a unit root",
        "adf_on_returns": adf_returns,
        "kpss_statistic": kpss_stat,
        "kpss_critical_values": _KPSS_CRITICAL,
        "kpss_verdict": "rejects stationarity" if kpss_rejects else "does not reject stationarity",
        "kpss_on_returns": kpss_returns,
        "joint_verdict": joint,
        "fracdiff_min_d": fracdiff_min_d,
        "half_life": half_life,
        "warnings": warnings,
        "window": {
            "instrument_id": payload.get("instrument_id"),
            "timeframe": payload.get("timeframe"),
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(
        detail,
        n=n,
        se=adf_level["se_rho"] if adf_level else None,
    )
