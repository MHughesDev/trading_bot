"""Do these two drift apart and come back, or just drift.

```json
{
  "name": "cointegration",
  "version": 1,
  "namespace": "analysis",
  "description": "Engle-Granger and Johansen trace tests on a pair, with the OU half-life and current z-score of the spread. Run it before building a pairs trade; a spread that is not cointegrated is a position that never closes.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "secondary_instrument_id", "timeframe", "start", "end"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string"},
      "secondary_instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string"},
      "end": {"type": "string"}
    }
  },
  "summary_template": "Engle-Granger {engle_granger_statistic} ({engle_granger_verdict} at 5%), Johansen trace {johansen_trace} ({johansen_verdict}); hedge ratio {hedge_ratio}, half-life {half_life} bars, spread currently {current_z} sigma from its mean",
  "detail_fields": [
    ["engle_granger_statistic", ""],
    ["engle_granger_verdict", ""],
    ["johansen_trace", ""],
    ["johansen_verdict", ""],
    ["hedge_ratio", ""],
    ["half_life", "bars"],
    ["current_z", "sigma"]
  ],
  "wall_seconds": 120,
  "series": [
    {"role": "primary"},
    {"role": "secondary", "instrument_arg": "secondary_instrument_id"}
  ]
}
```

Two tests, and they are not interchangeable
--------------------------------------------
* **Engle-Granger** regresses one log price on the other and tests the residual
  for a unit root. It is asymmetric — swapping which series is the dependent one
  gives a different answer — so both directions are run and both are reported.
  A pair that is cointegrated in one direction and not the other is not
  cointegrated; it is a pair where the test is at the edge of its power.
* **Johansen** is symmetric and tests the rank of the system directly. It is the
  one to believe when they disagree, and it is also the one whose critical values
  depend on a deterministic-term assumption that nobody states. This uses a
  constant inside the cointegrating relation, and says so.

Both use critical values embedded here — MacKinnon's for Engle-Granger,
Osterwald-Lenum's for the Johansen trace — rather than importing statsmodels.
See `apps/skill-runner/requirements.txt`.

The half-life is the number that decides the trade
--------------------------------------------------
Cointegration says the spread returns. It says nothing about when, and a
statistically perfect pair with a half-life of 400 bars is a position held for
months against financing, not a trade. The half-life is cross-checked against the
sample: a half-life longer than a quarter of the window has not actually been
observed reverting, and the tool says so rather than quoting it.

What this does not do
---------------------
It does not search for pairs. Testing one named pair is a hypothesis; scanning a
universe for the best p-value is a search, and it spends trials. That belongs in
a sweep with a registered family, not in a read tool.
"""

# MacKinnon (2010), Engle-Granger residual test, constant, no trend, two
# variables (N=2): tau(T) = tau_inf + tau1/T + tau2/T^2.
_EG_CRITICAL = {
    "1%": (-3.89644, -10.9519, -22.527),
    "5%": (-3.33613, -6.1101, -6.823),
    "10%": (-3.04445, -4.2412, -2.720),
}

# Osterwald-Lenum (1992), trace statistic, constant in the cointegrating
# relation. Indexed by the number of remaining series (n - r).
_TRACE_CRITICAL = {
    1: {"90%": 6.50, "95%": 8.18, "99%": 11.65},
    2: {"90%": 15.66, "95%": 17.95, "99%": 23.52},
}


def _eg_critical(n):
    return {lvl: a + b / n + c / (n * n) for lvl, (a, b, c) in _EG_CRITICAL.items()}


def _adf_on_residual(e, lag):
    """The ADF regression without a constant — the residual is mean zero by
    construction, and including a constant would cost a degree of freedom to
    estimate a number that is known."""
    de = np.diff(e)
    total = de.size
    if total <= lag + 3:
        return None
    target = de[lag:]
    columns = [e[lag:-1]]
    for i in range(1, lag + 1):
        columns.append(de[lag - i : total - i])
    x = np.column_stack(columns)
    coef, *_ = np.linalg.lstsq(x, target, rcond=None)
    resid = target - x @ coef
    dof = x.shape[0] - x.shape[1]
    if dof <= 0:
        return None
    sigma2 = float(resid @ resid) / dof
    try:
        covariance = sigma2 * np.linalg.inv(x.T @ x)
    except np.linalg.LinAlgError:
        return None
    se = math.sqrt(max(covariance[0, 0], 0.0))
    return float(coef[0] / se) if se > 0 else None


def _engle_granger(y, x, lag):
    """Regress y on x with a constant, then test the residual."""
    design = np.column_stack([np.ones(x.size), x])
    coef, *_ = np.linalg.lstsq(design, y, rcond=None)
    residual = y - design @ coef
    statistic = _adf_on_residual(residual, lag)
    return {
        "intercept": float(coef[0]),
        "hedge_ratio": float(coef[1]),
        "statistic": statistic,
        "residual": residual,
    }


def _johansen_trace(data, lag=1):
    """Johansen's trace statistic by reduced-rank regression.

    The standard three-step construction: regress the differences and the lagged
    level on the short-run terms, take the canonical correlations between the two
    sets of residuals, and form the trace from their eigenvalues.
    """
    n_obs, n_series = data.shape
    if n_obs < 50 * n_series:
        return None

    dx = np.diff(data, axis=0)
    level = data[lag:-1] if lag > 0 else data[:-1]
    target = dx[lag:]
    if target.shape[0] < 20:
        return None

    short_run = [np.ones((target.shape[0], 1))]
    for i in range(1, lag + 1):
        short_run.append(dx[lag - i : dx.shape[0] - i])
    z = np.hstack(short_run)

    def residualise(m):
        coef, *_ = np.linalg.lstsq(z, m, rcond=None)
        return m - z @ coef

    r0 = residualise(target)
    r1 = residualise(level)

    t = r0.shape[0]
    s00 = r0.T @ r0 / t
    s11 = r1.T @ r1 / t
    s01 = r0.T @ r1 / t
    try:
        s11_inv_root = np.linalg.inv(np.linalg.cholesky(s11))
        matrix = s11_inv_root @ s01.T @ np.linalg.inv(s00) @ s01 @ s11_inv_root.T
    except np.linalg.LinAlgError:
        return None

    eigenvalues = np.sort(np.real(np.linalg.eigvals(matrix)))[::-1]
    eigenvalues = np.clip(eigenvalues, 0.0, 1.0 - 1e-12)

    out = []
    for r in range(n_series):
        statistic = float(-t * np.sum(np.log(1.0 - eigenvalues[r:])))
        remaining = n_series - r
        out.append(
            {
                "null_rank": r,
                "trace": statistic,
                "critical_values": _TRACE_CRITICAL.get(remaining),
                "eigenvalue": float(eigenvalues[r]),
            }
        )
    return {
        "levels": out,
        "n_used": int(t),
        "deterministic": "constant in the cointegrating relation",
    }


def _half_life(spread):
    ds = np.diff(spread)
    x = np.column_stack([np.ones(ds.size), spread[:-1]])
    coef, *_ = np.linalg.lstsq(x, ds, rcond=None)
    theta = float(coef[1])
    if theta >= 0 or (1.0 + theta) <= 0:
        return None
    return float(-math.log(2.0) / math.log(1.0 + theta))


def run(payload: dict) -> dict:
    a = load(payload, "primary")
    b = load(payload, "secondary")

    # Inner join on timestamp. Forward-filling one series onto the other's index
    # would manufacture cointegration out of the fill itself, which is the most
    # common way a pairs backtest becomes a backtest of its own resampler.
    joined = a[["close"]].join(b[["close"]], how="inner", lsuffix="_a", rsuffix="_b")
    joined = joined.dropna()
    if len(joined) < 200:
        return refuse(
            f"only {len(joined)} bars are common to both series; a cointegration test below "
            f"200 has no power, and filling the gaps would manufacture the result",
            n_common=int(len(joined)),
            n_primary=int(len(a)),
            n_secondary=int(len(b)),
        )

    pa = pd.to_numeric(joined["close_a"], errors="coerce").to_numpy(dtype=float)
    pb = pd.to_numeric(joined["close_b"], errors="coerce").to_numpy(dtype=float)
    keep = np.isfinite(pa) & np.isfinite(pb) & (pa > 0) & (pb > 0)
    y, x = np.log(pa[keep]), np.log(pb[keep])
    n = int(y.size)

    lag = max(1, int(math.floor(4.0 * (n / 100.0) ** 0.25)))
    forward = _engle_granger(y, x, lag)
    reverse = _engle_granger(x, y, lag)
    critical = _eg_critical(n)

    def rejects(stat):
        return stat is not None and math.isfinite(stat) and stat < critical["5%"]

    forward_rejects = rejects(forward["statistic"])
    reverse_rejects = rejects(reverse["statistic"])

    johansen = _johansen_trace(np.column_stack([y, x]), lag=lag)
    johansen_rank0 = None
    johansen_verdict = "could not be computed"
    trace = None
    if johansen:
        johansen_rank0 = johansen["levels"][0]
        trace = johansen_rank0["trace"]
        cv = johansen_rank0["critical_values"]
        if cv and trace > cv["95%"]:
            rank1 = johansen["levels"][1]
            cv1 = rank1["critical_values"]
            if cv1 and rank1["trace"] > cv1["95%"]:
                johansen_verdict = "rank 2 — both series are stationary, which is not a pair"
            else:
                johansen_verdict = "rejects no cointegration at 5%"
        else:
            johansen_verdict = "does not reject no cointegration"

    spread = forward["residual"]
    sd = float(np.std(spread, ddof=1)) if spread.size > 2 else float("nan")
    current_z = float((spread[-1] - spread.mean()) / sd) if sd > 0 else None
    half_life = _half_life(spread)

    warnings = []
    if forward_rejects != reverse_rejects:
        warnings.append(
            "Engle-Granger rejects in one direction and not the other. The test is asymmetric, "
            "and a pair that only passes one way is a pair at the edge of the test's power — "
            "not a cointegrated one. Believe Johansen here."
        )
    if johansen_verdict.startswith("rejects") and not (forward_rejects or reverse_rejects):
        warnings.append(
            "Johansen rejects and Engle-Granger does not in either direction. Johansen is the "
            "symmetric test and is usually right, but this combination is also what a "
            "structural break inside the window looks like."
        )
    if half_life is not None and half_life > n / 4:
        warnings.append(
            f"the half-life of {half_life:.0f} bars is more than a quarter of the {n}-bar "
            f"window, so the reversion this relies on has not actually been observed happening. "
            f"A position on this spread is a financing cost with an option attached."
        )
    if half_life is None:
        warnings.append(
            "the spread's AR(1) coefficient is not mean-reverting, so there is no half-life to "
            "quote whatever the tests say."
        )

    detail = {
        "n_common_bars": n,
        "lags": lag,
        "engle_granger_statistic": forward["statistic"],
        "engle_granger_verdict": (
            "rejects no cointegration" if forward_rejects else "does not reject no cointegration"
        ),
        "engle_granger_reverse_statistic": reverse["statistic"],
        "engle_granger_reverse_verdict": (
            "rejects no cointegration" if reverse_rejects else "does not reject no cointegration"
        ),
        "engle_granger_critical_values": critical,
        "engle_granger_symmetric": forward_rejects == reverse_rejects,
        "hedge_ratio": forward["hedge_ratio"],
        "intercept": forward["intercept"],
        "johansen": johansen,
        "johansen_trace": trace,
        "johansen_verdict": johansen_verdict,
        "half_life": half_life,
        "spread_sd": sd,
        "current_z": current_z,
        "warnings": warnings,
        "window": {
            "instrument_id": payload.get("instrument_id"),
            "secondary_instrument_id": payload.get("secondary_instrument_id"),
            "timeframe": payload.get("timeframe"),
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(detail, n=n)
