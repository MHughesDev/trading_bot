"""Is this strategy's return a repackaging of the market, or something else.

```json
{
  "name": "factor_spanning_test",
  "version": 1,
  "namespace": "attribution",
  "description": "Regresses one recorded run's per-period returns on the platform's fixed crypto factor (BTC market return) with Newey-West errors, and reports the loading, the residual alpha with its t-statistic and the R-squared. The factor set is platform-fixed, not chosen per call, and it is a ONE-factor set today: funding-rate carry is not in it, so the verdict is about BTC beta only.",
  "input_schema": {
    "type": "object",
    "required": ["experiment_id", "run_id", "factor_instrument_id", "timeframe", "start", "end"],
    "additionalProperties": false,
    "properties": {
      "experiment_id": {"type": "string"},
      "run_id": {"type": "string", "description": "the recorded run to test"},
      "factor_instrument_id": {"type": "string", "enum": ["BTC-USD"],
                               "description": "fixed by the platform for the crypto set; there is one value so a report cannot choose its own factor"},
      "timeframe": {"type": "string", "description": "the run's own bar size; the factor is read at it"},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "nw_lags": {"type": "integer", "minimum": 0, "maximum": 50, "default": 5}
    }
  },
  "summary_template": "factor set {factor_set} ({n_factors} factor): beta {beta} (t {beta_t}), residual alpha {alpha_annualized} per year (t {alpha_t}), R2 {r2} over {n_aligned} periods; {verdict}",
  "detail_fields": [
    ["factor_set", ""],
    ["n_factors", ""],
    ["beta", ""],
    ["beta_t", ""],
    ["alpha_annualized", ""],
    ["alpha_t", ""],
    ["r2", ""],
    ["n_aligned", "periods"],
    ["verdict", ""]
  ],
  "wall_seconds": 120,
  "series": [
    {"role": "returns", "kind": "trial_returns"},
    {"role": "factor", "instrument_arg": "factor_instrument_id"}
  ]
}
```

What this answers, and what it does not
---------------------------------------
A strategy that is long crypto in a rising market has a good Sharpe and no edge:
its return is the market's, charged a fee. Regressing its returns on the market
separates the loading (what the market explains) from the residual (what it does
not), and the residual's t-statistic is the only part worth reporting as skill.

**The factor set is one factor, and the name says so.** AGENT-006 §5.3 asks for a
crypto proxy set of BTC beta *and* funding-rate carry. Funding carry needs a
funding series materialised for the template, and the platform has none (the
read route exists; no materialisation kind does). Doing BTC beta alone and
calling it a spanning test would report a one-factor result under a two-factor
name, so the set is named `btc_beta_v0` and `detail.factors_missing` says what is
not in it. Consequence for the verdict: `spanned_by_btc_beta` is a real finding,
but `alpha_beyond_btc_beta` is **not** "genuine" -- carry, a known source of
return for short-perpetual books, could still explain it.

The factor is fixed platform-wide: `factor_instrument_id` has one allowed value,
so a report cannot shop for the benchmark that makes its alpha significant.

Alignment, and why it is refused rather than padded
----------------------------------------------------
The run's periods and the factor's bars are joined on timestamp. Both carry the
bar timestamp the bar store uses, so a period and the factor return for the same
bar land on the same key. The join is inner: a period with no factor bar is
dropped and counted, and fewer than 80% aligned is a refusal, because a
regression over a minority of the run's periods is a regression about whichever
periods happened to have a factor.

Errors
------
Newey-West (Bartlett) heteroskedasticity-and-autocorrelation-consistent
covariance on the OLS coefficients, `nw_lags` lags (default 5, the spec's).
Plain OLS errors on overlapping or autocorrelated strategy returns overstate the
alpha's significance, which is how a spanned strategy acquires a t of 3.
"""


def _hac_ols(y, x, lags):
    """OLS of y on [1, x] with Newey-West covariance. Returns (coef, se, r2)."""
    n = y.size
    design = np.column_stack([np.ones(n), x])
    xtx_inv = np.linalg.inv(design.T @ design)
    coef = xtx_inv @ design.T @ y
    resid = y - design @ coef
    scores = design * resid[:, None]
    s = scores.T @ scores / n
    lags = int(max(0, min(lags, n - 2)))
    for lag in range(1, lags + 1):
        w = 1.0 - lag / (lags + 1.0)
        gamma = scores[lag:].T @ scores[:-lag] / n
        s = s + w * (gamma + gamma.T)
    cov = n * xtx_inv @ s @ xtx_inv
    se = np.sqrt(np.maximum(np.diag(cov), 0.0))
    ss_tot = float(np.sum((y - y.mean()) ** 2))
    r2 = 1.0 - float(np.sum(resid**2)) / ss_tot if ss_tot > 0 else float("nan")
    return coef, se, r2


def run(payload: dict) -> dict:
    run_id = str(payload.get("run_id") or "")
    lags = int(payload.get("nw_lags") if payload.get("nw_lags") is not None else 5)
    timeframe = str(payload.get("timeframe") or "")
    if payload.get("factor_instrument_id") != "BTC-USD":
        return refuse(
            "the crypto factor set is fixed to BTC-USD; a different factor is a different "
            "report, and letting the caller choose is how a spanned alpha finds a benchmark "
            "that does not span it",
            factor_instrument_id=payload.get("factor_instrument_id"),
        )

    family = load_family(payload, "returns")
    if "ts" not in family.columns:
        return refuse(
            "the recorded returns carry no timestamp, so they cannot be aligned to a factor",
            columns=[str(c) for c in family.columns],
        )
    mine = family[family["run_id"].astype(str) == run_id]
    if mine.empty:
        return refuse(
            f"run {run_id!r} is not in this experiment's recorded returns",
            known_runs=sorted({str(r) for r in family["run_id"].unique()})[:20],
        )
    y_ts = epoch_seconds(pd.DatetimeIndex(pd.to_datetime(mine["ts"], utc=True)))
    y_ret = pd.Series(pd.to_numeric(mine["ret"], errors="coerce").to_numpy(dtype=float), index=y_ts)
    y_ret = y_ret[~y_ret.index.duplicated(keep="first")].dropna()

    bars = load(payload, "factor")
    close = pd.to_numeric(bars["close"], errors="coerce").to_numpy(dtype=float)
    stamps = epoch_seconds(bars.index)
    ok = np.isfinite(close) & (close > 0)
    close, stamps = close[ok], stamps[ok]
    if close.size < 3:
        return refuse("the factor series has fewer than three usable bars")
    f_ret = pd.Series(np.diff(np.log(close)), index=stamps[1:])
    f_ret = f_ret[~f_ret.index.duplicated(keep="first")]

    joined = pd.concat([y_ret.rename("y"), f_ret.rename("f")], axis=1, join="inner").dropna()
    n_run = int(y_ret.size)
    n = int(len(joined))
    if n < 60 or n < 0.8 * n_run:
        return refuse(
            f"only {n} of the run's {n_run} periods align with a factor bar; a regression over "
            f"a minority of the run is about the periods that happened to have a factor",
            n_aligned=n,
            n_run_periods=n_run,
        )
    y = joined["y"].to_numpy(dtype=float)
    f = joined["f"].to_numpy(dtype=float)
    if float(np.std(f)) <= 0.0:
        return refuse("the factor has no variance over the aligned periods")

    coef, se, r2 = _hac_ols(y, f, lags)
    alpha, beta = float(coef[0]), float(coef[1])
    alpha_t = alpha / float(se[0]) if se[0] > 0 else float("nan")
    beta_t = beta / float(se[1]) if se[1] > 0 else float("nan")
    ppy = bars_per_year(timeframe)
    alpha_ann = alpha * ppy if np.isfinite(ppy) else None

    if not np.isfinite(alpha_t):
        verdict = "inconclusive"
    elif abs(alpha_t) < 2.0:
        verdict = "spanned_by_btc_beta"
    elif alpha_t > 0:
        verdict = "alpha_beyond_btc_beta_carry_untested"
    else:
        verdict = "negative_alpha_after_btc_beta"

    detail = {
        "factor_set": "btc_beta_v0",
        "n_factors": 1,
        "factors_missing": ["funding_rate_carry"],
        "factors_missing_note": "AGENT-006 §5.3's crypto proxy set also has funding-rate carry; no funding series is materialisable for a template, so it is not tested",
        "beta": beta,
        "beta_t": beta_t,
        "beta_se": float(se[1]),
        "alpha_per_period": alpha,
        "alpha_se": float(se[0]),
        "alpha_t": alpha_t,
        "alpha_annualized": alpha_ann,
        "r2": r2,
        "n_aligned": n,
        "n_run_periods": n_run,
        "nw_lags": lags,
        "periods_per_year": ppy if np.isfinite(ppy) else None,
        "verdict": verdict,
        "verdict_rule": "|alpha t| < 2 spans; the threshold is the conventional two and is not a parameter",
        "run_id": run_id,
        "window": {
            "timeframe": timeframe,
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(detail, n=n, se=float(se[0]))
