"""What kind of series is this, before anyone trades it.

```json
{
  "name": "describe_returns",
  "version": 1,
  "namespace": "analysis",
  "description": "Moments with standard errors, tail index, autocorrelation and ARCH diagnostics for one instrument's returns. Run this before any strategy work on an unfamiliar series.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "timeframe", "start", "end"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string", "description": "e.g. BTC-USD"},
      "timeframe": {"type": "string", "description": "e.g. 1m, 1h, 1d"},
      "start": {"type": "string", "description": "RFC 3339, inclusive"},
      "end": {"type": "string", "description": "RFC 3339, exclusive"},
      "lags": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20,
               "description": "Ljung-Box and ARCH-LM lag count"}
    }
  },
  "summary_template": "n={n_bars} bars, mean {mean}, sd {sd}, skew {skew}, excess kurtosis {excess_kurtosis}, tail index {hill_alpha}, max drawdown {max_drawdown}; returns {autocorr_verdict}, volatility {arch_verdict}",
  "detail_fields": [
    ["n_bars", "bars"],
    ["mean", "ret/bar"],
    ["sd", "ret/bar"],
    ["skew", ""],
    ["excess_kurtosis", ""],
    ["hill_alpha", ""],
    ["max_drawdown", ""],
    ["autocorr_verdict", ""],
    ["arch_verdict", ""]
  ],
  "wall_seconds": 120,
  "series": [{"role": "primary"}]
}
```

Why these and not a longer list
-------------------------------
Each diagnostic here answers a question that changes what may be done next, and
nothing here is reported for completeness:

* **Excess kurtosis and the Hill index** say whether a variance exists in any
  useful sense. With `alpha` below 2 the sample variance does not converge, and
  every Sharpe ratio computed downstream is an average of a quantity with no
  population value — the number will still print, which is the danger.
* **Ljung-Box on `r`** is the only honest predictability screen here, and it is
  reported *with* its multiple-lag correction rather than as "lag 3 was
  significant".
* **Ljung-Box on `r²` and ARCH-LM** say whether volatility clusters. If it does,
  an i.i.d. bootstrap anywhere downstream is wrong, and every confidence band
  built from one is too tight.
* **Jump share** separates diffusive variance from jumps. A strategy whose edge
  lives entirely in the jump component is a strategy whose backtest depends on
  fills that would not have happened.

Standard errors, not just point estimates: a skewness of -0.4 on 300 bars and one
on 300,000 are different claims, and only the second is a claim.
"""


def _moment_diagnostics(r, n):
    """Skewness and excess kurtosis with the exact small-sample errors.

    The textbook `sqrt(6/n)` and `sqrt(24/n)` are asymptotic and badly optimistic
    at the sample sizes an agent will actually ask about; these are the
    finite-sample forms.
    """
    m = r.mean()
    s2 = ((r - m) ** 2).mean()
    if s2 <= 0:
        return None
    s = math.sqrt(s2)
    g1 = float(((r - m) ** 3).mean() / s**3)
    g2 = float(((r - m) ** 4).mean() / s**4 - 3.0)

    if n > 3:
        se_skew = math.sqrt(6.0 * n * (n - 1) / ((n - 2) * (n + 1) * (n + 3)))
    else:
        se_skew = float("nan")
    if n > 5:
        se_kurt = 2.0 * se_skew * math.sqrt((n * n - 1.0) / ((n - 3.0) * (n + 5.0)))
    else:
        se_kurt = float("nan")
    return g1, g2, se_skew, se_kurt


def _ljung_box(r, lags):
    """Q statistic and its p-value, testing all lags jointly.

    Jointly is the point. Scanning lags one at a time and reporting the smallest
    p-value is a multiple-comparison error that reliably finds predictability in
    noise.
    """
    n = r.size
    lags = int(min(lags, max(1, n // 5)))
    q = 0.0
    rhos = []
    for k in range(1, lags + 1):
        rho = autocorr(r, k)
        if not math.isfinite(rho):
            continue
        rhos.append(rho)
        q += rho * rho / (n - k)
    q *= n * (n + 2)
    p = float(st.chi2.sf(q, len(rhos))) if _HAVE_SCIPY and rhos else float("nan")
    return {"q": float(q), "lags": len(rhos), "p_value": p, "acf": rhos[:10]}


def _arch_lm(r, lags):
    """Engle's LM test: regress squared returns on their own lags."""
    e2 = (r - r.mean()) ** 2
    n = e2.size
    lags = int(min(lags, max(1, n // 10)))
    if n <= lags + 2:
        return {"lm": None, "lags": lags, "p_value": None}
    y = e2[lags:]
    x = np.column_stack([np.ones(y.size)] + [e2[lags - k : n - k] for k in range(1, lags + 1)])
    coef, *_ = np.linalg.lstsq(x, y, rcond=None)
    resid = y - x @ coef
    ss_tot = float(((y - y.mean()) ** 2).sum())
    r2 = 1.0 - float((resid**2).sum()) / ss_tot if ss_tot > 0 else 0.0
    lm = y.size * r2
    p = float(st.chi2.sf(lm, lags)) if _HAVE_SCIPY else float("nan")
    return {"lm": float(lm), "lags": lags, "p_value": p, "r_squared": float(r2)}


def _hill(r):
    """Tail index from the upper order statistics of |r|.

    `k` is the standard `sqrt(n)` rule rather than an optimised choice: the Hill
    plot is famously unstable in `k`, and an agent picking `k` to get the alpha
    it wants is a real failure mode. A fixed rule is worse statistics and better
    evidence.
    """
    a = np.sort(np.abs(r))[::-1]
    a = a[a > 0]
    n = a.size
    if n < 30:
        return {"alpha": None, "k": 0, "note": "fewer than 30 non-zero returns"}
    k = int(max(10, min(round(math.sqrt(n)), n - 1)))
    tail = np.log(a[:k]) - math.log(a[k])
    inv = float(tail.mean())
    alpha = 1.0 / inv if inv > 0 else None
    return {
        "alpha": alpha,
        "k": k,
        # The asymptotic error, which is the honest width on a number this
        # sensitive to k.
        "se": (alpha / math.sqrt(k)) if alpha else None,
    }


def _jump_share(r):
    """The fraction of quadratic variation that is not diffusive.

    Realised variance counts everything; bipower variation is robust to jumps.
    The gap is the jump component, floored at zero because the estimator is noisy
    and a negative jump share is an artefact, not a finding.
    """
    n = r.size
    if n < 10:
        return {"rv": None, "bv": None, "jump_share": None}
    rv = float((r**2).sum())
    mu1 = math.sqrt(2.0 / math.pi)
    bv = float((np.abs(r[1:]) * np.abs(r[:-1])).sum() / (mu1**2))
    share = max(0.0, (rv - bv) / rv) if rv > 0 else None
    return {"rv": rv, "bv": bv, "jump_share": share}


def run(payload: dict) -> dict:
    frame = load(payload, "primary")
    r = close_returns(frame)
    n = int(r.size)
    if n < 30:
        return refuse(
            "a return series of fewer than 30 bars supports no moment estimate worth "
            "reporting; widen the window",
            n_bars=n,
        )

    lags = int(payload.get("lags") or 20)
    base = summary_stats(r)
    moments = _moment_diagnostics(r, n)
    if moments is None:
        return refuse("the series has zero variance over this window", n_bars=n)
    skew, excess_kurtosis, se_skew, se_kurt = moments

    lb_r = _ljung_box(r, lags)
    lb_r2 = _ljung_box((r - r.mean()) ** 2, lags)
    arch = _arch_lm(r, lags)
    hill = _hill(r)
    jumps = _jump_share(r)
    equity = np.exp(np.cumsum(r))
    dd = max_drawdown(equity)

    # The two verdicts the summary line carries. Written from the p-values rather
    # than left to the reader, because the summary is the only thing a flat-tier
    # model reliably reads and "p=0.0312" in it invites the reader to pick a
    # threshold after seeing the number.
    def verdict(p, yes, no):
        if p is None or not math.isfinite(p):
            return "undetermined"
        return yes if p < 0.05 else no

    autocorr_verdict = verdict(lb_r["p_value"], "autocorrelated", "look serially independent")
    arch_verdict = verdict(arch["p_value"], "clusters", "shows no ARCH effect")

    alpha = hill.get("alpha")
    detail = {
        "n_bars": n,
        "mean": base["mean"],
        "sd": base["sd"],
        "se_mean_hac": base["se"],
        "se_mean_iid": base["se_iid"],
        "skew": skew,
        "se_skew": se_skew,
        "excess_kurtosis": excess_kurtosis,
        "se_excess_kurtosis": se_kurt,
        "hill_alpha": alpha,
        "hill": hill,
        "max_drawdown": dd,
        "ljung_box_returns": lb_r,
        "ljung_box_squared_returns": lb_r2,
        "arch_lm": arch,
        "jumps": jumps,
        "autocorr_verdict": autocorr_verdict,
        "arch_verdict": arch_verdict,
        "window": {
            "instrument_id": payload.get("instrument_id"),
            "timeframe": payload.get("timeframe"),
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }

    # Jarque-Bera, reported but not used for a verdict: it rejects normality on
    # essentially every financial series ever recorded, so a "significant" JB is
    # not news. The kurtosis and the tail index are what carry information.
    if _HAVE_SCIPY:
        jb = n / 6.0 * (skew**2 + (excess_kurtosis**2) / 4.0)
        detail["jarque_bera"] = {"statistic": float(jb), "p_value": float(st.chi2.sf(jb, 2))}

    # The caution that changes what may be done next, rather than a footnote.
    warnings = []
    if alpha is not None and alpha < 2.0:
        warnings.append(
            f"tail index {alpha:.2f} < 2: the population variance does not exist, so any Sharpe "
            f"ratio computed on this series is an average of a quantity with no limit. Use "
            f"median-based statistics and quantile targets instead."
        )
    if arch_verdict == "clusters":
        warnings.append(
            "volatility clusters, so an i.i.d. bootstrap anywhere downstream will produce "
            "confidence bands that are too tight. Use the block bootstrap."
        )
    if jumps.get("jump_share") is not None and jumps["jump_share"] > 0.3:
        warnings.append(
            f"{jumps['jump_share']:.0%} of quadratic variation is jumps, so an edge here may "
            f"depend on fills at prices that were never resting."
        )
    detail["warnings"] = warnings

    return result(detail, n=n, se=base["se"])
