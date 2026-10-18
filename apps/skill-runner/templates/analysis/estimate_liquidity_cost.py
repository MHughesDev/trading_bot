"""What it costs to get in and out, from bars alone.

```json
{
  "name": "estimate_liquidity_cost",
  "version": 1,
  "namespace": "analysis",
  "description": "Effective spread by four bar-based estimators (EDGE, Roll, Corwin-Schultz, Abdi-Ranaldo) plus Amihud illiquidity and a square-root impact curve. Use it to find the turnover at which a strategy's edge is eaten by its own costs.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "timeframe", "start", "end"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "notional_usd": {"type": "number", "minimum": 0, "default": 10000,
                       "description": "order size for the impact estimate"}
    }
  },
  "summary_template": "effective spread {spread_bps} bps (consensus of {n_estimators} estimators, spread between them {estimator_disagreement_bps} bps); Amihud {amihud}; a {notional_usd} order costs about {impact_bps} bps of impact, so a round trip needs {breakeven_bps} bps of edge",
  "detail_fields": [
    ["spread_bps", "bps"],
    ["n_estimators", ""],
    ["estimator_disagreement_bps", "bps"],
    ["amihud", ""],
    ["notional_usd", "USD"],
    ["impact_bps", "bps"],
    ["breakeven_bps", "bps"]
  ],
  "wall_seconds": 120,
  "series": [{"role": "primary"}]
}
```

Four estimators because each one breaks somewhere
--------------------------------------------------
None of these observe a quote. They infer the spread from what bars do, and each
inference fails on a different kind of data:

* **Roll (1984)** reads the spread out of the negative first-order
  autocovariance of returns. When that autocovariance is *positive* — which
  happens whenever the series trends inside the bar — Roll has no real solution
  and the usual implementation quietly returns zero. A zero spread is not a
  finding; it is the estimator refusing, and it is reported as `None` here.
* **Corwin-Schultz (2012)** uses two-day high-low ranges. It assumes the true
  high and low are observed, so it breaks on exactly the stale-quote data that
  `estimate_volatility` warns about.
* **Abdi-Ranaldo (2017)** uses the close against the mid of the high and low. It
  is more robust than Corwin-Schultz on gappy data and is biased on very short
  bars.

* **EDGE (Ardia, Guidotti & Kroencke, 2024)** is a GMM estimator over open, high,
  low and close of consecutive bars. It uses the moment conditions the paper
  derives from the open-to-mid, close-to-mid and overnight log-price steps, each
  demeaned by the share of bars that traded, and combines the two resulting
  squared-spread estimates by inverse-variance weight. It is the one estimator
  here that uses the open, and it is the least biased when the high and low are
  observed at discrete times. Its squared-spread estimate is **negative** when
  the bounce is smaller than sampling noise; that is the estimator declining,
  reported as `None`, and it is never floored to zero.

  History, because the number is only worth trusting with it: the first version
  of this estimator (OQ-15) was written without the paper to hand and returned a
  negative variance against a series whose spread is known exactly, so it was
  withheld. This version follows the published moment conditions and is held to
  the same fixture by `the_cost_estimators_recover_the_fixtures_known_spread`,
  which checks it against two series with different known spreads.

The consensus is the **median**, not the mean: these disagree by large factors on
bad data, and a mean is dragged by whichever one broke. `estimator_disagreement_bps`
is the spread between the surviving estimates, and it is in the summary because a
consensus of 8 bps from estimates of {7, 8, 9} and one from {1, 8, 40} are not
the same claim.

A caveat that belongs on the number, not in a footnote: all three are below their
own noise floor when the spread is small relative to bar volatility. At a
realistic 8 bps on hourly crypto the bounce moves the first return autocovariance
by about -1.6e-7 against a sampling error near 4e-7, so Roll declines and the
others are noise. These estimators want daily bars, or a genuinely wide market.

Impact
------
Square-root law: `impact ≈ Y * sigma * sqrt(Q / V)`, with `Y ≈ 1` — the standard
calibration across markets, and the reason the number is reported as an order of
magnitude rather than to three figures. `breakeven_bps` is the full round trip:
two spreads plus two impacts. That is the edge a strategy has to have *before*
it has any.
"""

_BPS = 10_000.0


def _roll(r):
    """Roll (1984): spread = 2*sqrt(-cov(r_t, r_{t-1})).

    Returns `None` when the autocovariance is positive, which is the estimator
    having no solution. Implementations that clamp to zero report a zero spread,
    and a zero spread in a cost model makes any strategy profitable.
    """
    if r.size < 30:
        return None
    covariance = float(np.cov(r[:-1], r[1:], ddof=1)[0, 1])
    if covariance >= 0:
        return None
    return 2.0 * math.sqrt(-covariance)


def _corwin_schultz(h, l):
    """Corwin-Schultz (2012) two-period high-low estimator."""
    n = h.size
    if n < 30:
        return None
    with np.errstate(divide="ignore", invalid="ignore"):
        beta = np.log(h[1:] / l[1:]) ** 2 + np.log(h[:-1] / l[:-1]) ** 2
        h2 = np.maximum(h[1:], h[:-1])
        l2 = np.minimum(l[1:], l[:-1])
        gamma = np.log(h2 / l2) ** 2

    denominator = 3.0 - 2.0 * math.sqrt(2.0)
    alpha = (np.sqrt(2.0 * beta) - np.sqrt(beta)) / denominator - np.sqrt(
        gamma / denominator
    )
    alpha = np.where(np.isfinite(alpha), alpha, np.nan)
    spread = 2.0 * (np.exp(alpha) - 1.0) / (1.0 + np.exp(alpha))
    # Negative estimates are set to zero per the original paper's own
    # recommendation, and the fraction that were negative is reported, because a
    # series where half the estimates are negative is a series this estimator
    # does not fit.
    negative = float(np.nanmean(spread < 0)) if np.isfinite(spread).any() else None
    spread = np.where(spread < 0, 0.0, spread)
    value = float(np.nanmean(spread)) if np.isfinite(spread).any() else None
    return None if value is None else {"spread": value, "negative_fraction": negative}


def _abdi_ranaldo(h, l, c):
    """Abdi-Ranaldo (2017): close against the high-low mid."""
    n = c.size
    if n < 30:
        return None
    with np.errstate(divide="ignore", invalid="ignore"):
        eta = (np.log(h) + np.log(l)) / 2.0
        gap = (np.log(c[:-1]) - eta[:-1]) * (np.log(c[:-1]) - eta[1:])
    gap = gap[np.isfinite(gap)]
    if gap.size == 0:
        return None
    value = 4.0 * float(np.mean(gap))
    return math.sqrt(value) if value > 0 else None


def _edge(o, h, l, c):
    """EDGE (Ardia, Guidotti & Kroencke 2024), the efficient GMM combination.

    Works on consecutive bars, so it takes the log of each series and pairs bar
    `t` with bar `t-1`. Returns the spread as a fraction of price, or `None`
    when the estimate does not exist: too few bars, no bar that traded, or a
    negative squared spread (the bounce is below the noise, so there is no
    real-valued spread to report).
    """
    if o.size < 30:
        return None
    lo, lh, ll, lc = np.log(o), np.log(h), np.log(l), np.log(c)
    h1, l1, c1 = lh[:-1], ll[:-1], lc[:-1]
    m = (lh + ll) / 2.0
    m1 = m[:-1]
    o_, h_, l_, c_, m_ = lo[1:], lh[1:], ll[1:], lc[1:], m[1:]
    r1 = m_ - o_
    r2 = o_ - m1
    r3 = m_ - c1
    r4 = c1 - m1
    r5 = o_ - c1
    # tau: the bar did not just repeat itself (a flat bar carries no information)
    tau = ((h_ != l_) | (l_ != c1)).astype(float)
    pt = float(tau.mean())
    po = float((tau * (o_ != h_)).mean() + (tau * (o_ != l_)).mean())
    pc = float((tau * (c1 != h1)).mean() + (tau * (c1 != l1)).mean())
    if pt == 0.0 or po == 0.0 or pc == 0.0:
        return None
    d1 = r1 - float(r1.mean()) / pt * tau
    d3 = r3 - float(r3.mean()) / pt * tau
    d5 = r5 - float(r5.mean()) / pt * tau
    x1 = -4.0 / po * d1 * r2 - 4.0 / pc * d3 * r4
    x2 = -4.0 / po * d1 * r5 - 4.0 / pc * d5 * r4
    e1, e2 = float(x1.mean()), float(x2.mean())
    v1 = float((x1**2).mean()) - e1**2
    v2 = float((x2**2).mean()) - e2**2
    if not (v1 + v2) > 0.0:
        return None
    w1 = v2 / (v1 + v2)
    s2 = w1 * e1 + (1.0 - w1) * e2
    return math.sqrt(s2) if s2 > 0.0 else None


def run(payload: dict) -> dict:
    frame = load(payload, "primary")
    o = pd.to_numeric(frame["open"], errors="coerce").to_numpy(dtype=float)
    h = pd.to_numeric(frame["high"], errors="coerce").to_numpy(dtype=float)
    l = pd.to_numeric(frame["low"], errors="coerce").to_numpy(dtype=float)
    c = pd.to_numeric(frame["close"], errors="coerce").to_numpy(dtype=float)
    v = pd.to_numeric(frame["volume"], errors="coerce").to_numpy(dtype=float)

    ok = np.isfinite(o) & np.isfinite(h) & np.isfinite(l) & np.isfinite(c)
    ok &= (o > 0) & (h > 0) & (l > 0) & (c > 0) & (h >= l)
    o, h, l, c = o[ok], h[ok], l[ok], c[ok]
    v = np.where(np.isfinite(v[ok]), v[ok], 0.0)
    n = int(c.size)
    if n < 100:
        return refuse(
            "a bar-based spread estimate needs at least 100 bars; below that the estimators "
            "disagree by more than the number they are estimating",
            n_bars=n,
        )

    r = np.diff(np.log(c))

    estimates = {}
    roll = _roll(r)
    if roll is not None:
        estimates["roll"] = roll
    cs = _corwin_schultz(h, l)
    if cs is not None:
        estimates["corwin_schultz"] = cs["spread"]
    ar = _abdi_ranaldo(h, l, c)
    if ar is not None:
        estimates["abdi_ranaldo"] = ar
    edge = _edge(o, h, l, c)
    if edge is not None:
        estimates["edge"] = edge
    refused = {
        "edge": edge is None,
        "roll": roll is None,
        "corwin_schultz": cs is None,
        "abdi_ranaldo": ar is None,
    }

    if not estimates:
        return refuse(
            "every spread estimator declined on this series. That is itself a finding: the "
            "bars' open, high, low and close do not carry a recoverable spread, so no cost "
            "model built from them can be trusted",
            n_bars=n,
            refused=refused,
        )

    values = np.array(sorted(estimates.values()))
    # Median, not mean: on bad data these disagree by factors, and a mean is
    # dragged by whichever estimator broke.
    consensus = float(np.median(values))
    disagreement = float(values.max() - values.min())

    # Amihud illiquidity: |return| per unit of dollar volume. Its level is not
    # comparable across instruments; its variation over time is.
    dollar_volume = v[1:] * c[1:]
    with np.errstate(divide="ignore", invalid="ignore"):
        amihud_series = np.where(dollar_volume > 0, np.abs(r) / dollar_volume, np.nan)
    amihud = float(np.nanmean(amihud_series)) if np.isfinite(amihud_series).any() else None

    # Square-root impact. Y ~ 1 is the standard cross-market calibration, which
    # is why this is an order of magnitude and not a quote.
    notional = float(payload.get("notional_usd") or 10_000.0)
    sigma = float(np.std(r, ddof=1)) if n > 2 else float("nan")
    median_dollar_volume = (
        float(np.median(dollar_volume[dollar_volume > 0])) if np.any(dollar_volume > 0) else None
    )
    if median_dollar_volume and sigma > 0:
        impact = 1.0 * sigma * math.sqrt(notional / median_dollar_volume)
    else:
        impact = None

    spread_bps = consensus * _BPS
    impact_bps = impact * _BPS if impact is not None else None
    breakeven_bps = (
        2.0 * spread_bps + 2.0 * impact_bps if impact_bps is not None else 2.0 * spread_bps
    )

    warnings = []
    if edge is None:
        warnings.append(
            "EDGE declined: its squared-spread estimate was not positive (the bounce is below "
            "the sampling noise) or too few bars traded. It is not floored to zero."
        )
    if roll is None:
        warnings.append(
            "Roll's estimator has no solution here: the first-order return autocovariance is "
            "positive, which means the series trends inside the bar. Implementations that "
            "return zero in this case would have reported a zero spread."
        )
    if cs and cs["negative_fraction"] and cs["negative_fraction"] > 0.4:
        warnings.append(
            f"{cs['negative_fraction']:.0%} of Corwin-Schultz estimates were negative and were "
            f"floored at zero. Above about 40% the estimator does not fit this series and its "
            f"contribution to the consensus should be discounted."
        )
    if disagreement > consensus:
        warnings.append(
            f"the estimators disagree by more than the consensus itself ({disagreement * _BPS:.1f} "
            f"bps against {spread_bps:.1f}). Treat the spread as unknown to within a factor of "
            f"two and size any cost assumption at the pessimistic end."
        )
    if median_dollar_volume is None:
        warnings.append(
            "the volume column is empty or zero, so Amihud and the impact estimate are "
            "unavailable. `breakeven_bps` is spread only and is therefore a lower bound."
        )

    detail = {
        "n_bars": n,
        "estimators_bps": {k: val * _BPS for k, val in estimates.items()},
        "estimators_declined": [k for k, was in refused.items() if was],
        "n_estimators": len(estimates),
        "spread_bps": spread_bps,
        "estimator_disagreement_bps": disagreement * _BPS,
        "corwin_schultz_negative_fraction": cs["negative_fraction"] if cs else None,
        "amihud": amihud,
        "median_dollar_volume": median_dollar_volume,
        "volatility_per_bar": sigma,
        "notional_usd": notional,
        "impact_bps": impact_bps,
        "breakeven_bps": breakeven_bps,
        "breakeven_note": "two spreads plus two impacts: the edge a round trip needs before it has any",
        "warnings": warnings,
        "window": {
            "instrument_id": payload.get("instrument_id"),
            "timeframe": payload.get("timeframe"),
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(detail, n=n)
