"""Are these two return distributions the same, and if not, where do they differ.

```json
{
  "name": "compare_distributions",
  "version": 1,
  "namespace": "analysis",
  "description": "Kolmogorov-Smirnov, Wasserstein distance and per-quantile deltas between two instruments' return distributions. Use it to check whether a strategy fitted on one will behave the same on the other.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "secondary_instrument_id", "timeframe", "start", "end", "seed"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string"},
      "secondary_instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "seed": {"type": "integer"},
      "standardise": {"type": "boolean", "default": true,
                      "description": "divide each series by its own volatility first"}
    }
  },
  "summary_template": "KS {ks_statistic} (p={ks_p_value}), Wasserstein {wasserstein} ({wasserstein_in_sigma} sigma); the distributions {verdict}; the largest gap is at the {worst_quantile} quantile",
  "detail_fields": [
    ["ks_statistic", ""],
    ["ks_p_value", ""],
    ["wasserstein", ""],
    ["wasserstein_in_sigma", "sigma"],
    ["verdict", ""],
    ["worst_quantile", ""]
  ],
  "wall_seconds": 120,
  "series": [
    {"role": "primary"},
    {"role": "secondary", "instrument_arg": "secondary_instrument_id"}
  ]
}
```

Standardise first, by default
------------------------------
Two instruments with different volatilities have different return distributions,
and saying so is not a finding. The question a strategy transfer actually asks is
whether the *shapes* match — tail weight, asymmetry, the mass near zero — and that
question needs each series divided by its own scale first. `standardise: false` is
available for the rarer case where the absolute scale is the point.

Three measures because they see different things
--------------------------------------------------
* **KS** is the largest vertical gap between the two empirical CDFs. It is most
  sensitive near the middle of the distribution and nearly blind in the tails,
  which is the opposite of where the risk is.
* **Wasserstein** is the average horizontal distance, so it does see the tails,
  and it is in the units of the data — which is why it is also reported in
  sigmas, since "0.0004" means nothing without a scale.
* **Quantile deltas** are the only one that says *where*. A KS statistic of 0.08
  from a fatter left tail and one from a shifted mean are the same number and
  completely different problems for a strategy.

The p-value is not the answer
------------------------------
On tens of thousands of bars KS rejects equality for differences far too small to
matter, and on a few hundred it fails to reject differences that matter a great
deal. So the verdict is built from the Wasserstein distance measured in sigmas —
an effect size — and the p-value is reported beside it rather than in place of it.
"""

# Where the verdict's thresholds come from: a Wasserstein distance of a tenth of
# a standard deviation is roughly the point at which a strategy's fill
# distribution changes enough to matter, and a quarter is where the two series
# are no longer the same kind of thing. Stated here so they can be argued with
# rather than buried in an if.
_SIMILAR_SIGMA = 0.10
_DIFFERENT_SIGMA = 0.25

_QUANTILES = [0.01, 0.05, 0.10, 0.25, 0.50, 0.75, 0.90, 0.95, 0.99]


def _ks(a, b):
    """Two-sample KS statistic and its asymptotic p-value."""
    combined = np.sort(np.concatenate([a, b]))
    cdf_a = np.searchsorted(np.sort(a), combined, side="right") / a.size
    cdf_b = np.searchsorted(np.sort(b), combined, side="right") / b.size
    statistic = float(np.max(np.abs(cdf_a - cdf_b)))
    effective = math.sqrt(a.size * b.size / (a.size + b.size))
    lam = (effective + 0.12 + 0.11 / effective) * statistic
    # Kolmogorov's series. Converges quickly; 100 terms is far more than needed.
    total = 0.0
    for k in range(1, 101):
        total += ((-1.0) ** (k - 1)) * math.exp(-2.0 * (k**2) * (lam**2))
    p = max(0.0, min(1.0, 2.0 * total))
    return statistic, p


def _wasserstein(a, b):
    """First Wasserstein distance: the mean absolute gap between quantiles."""
    grid = np.linspace(0.0, 1.0, 1001)[1:-1]
    qa = np.quantile(a, grid)
    qb = np.quantile(b, grid)
    return float(np.mean(np.abs(qa - qb)))


def run(payload: dict) -> dict:
    a = load(payload, "primary")
    b = load(payload, "secondary")
    ra = close_returns(a)
    rb = close_returns(b)
    if ra.size < 200 or rb.size < 200:
        return refuse(
            "comparing distributions needs at least 200 observations of each",
            n_primary=int(ra.size),
            n_secondary=int(rb.size),
        )

    standardise = payload.get("standardise")
    standardise = True if standardise is None else bool(standardise)
    sigma_a = float(np.std(ra, ddof=1))
    sigma_b = float(np.std(rb, ddof=1))
    if standardise:
        if sigma_a <= 0 or sigma_b <= 0:
            return refuse("one of the series has zero variance and cannot be standardised")
        xa, xb = ra / sigma_a, rb / sigma_b
        scale = 1.0
    else:
        xa, xb = ra, rb
        # The reference scale for reporting in sigmas is the *primary*: the
        # question is usually "will what I fitted here work there".
        scale = sigma_a

    ks_statistic, ks_p = _ks(xa, xb)
    wasserstein = _wasserstein(xa, xb)
    wasserstein_sigma = wasserstein / scale if scale > 0 else None

    qa = np.quantile(xa, _QUANTILES)
    qb = np.quantile(xb, _QUANTILES)
    deltas = []
    for q, va, vb in zip(_QUANTILES, qa, qb):
        deltas.append(
            {
                "quantile": q,
                "primary": float(va),
                "secondary": float(vb),
                "delta": float(vb - va),
                "delta_in_sigma": float((vb - va) / scale) if scale > 0 else None,
            }
        )
    worst = max(deltas, key=lambda d: abs(d["delta"]))

    if wasserstein_sigma is None:
        verdict = "could not be compared on a common scale"
    elif wasserstein_sigma < _SIMILAR_SIGMA:
        verdict = "are close enough that a strategy should transfer"
    elif wasserstein_sigma < _DIFFERENT_SIGMA:
        verdict = "differ enough that a strategy's fill and stop behaviour will change"
    else:
        verdict = "are not the same kind of series"

    # Tail weight separately, because that is what KS is worst at and what a
    # strategy's drawdown depends on most.
    tail_a = float(np.mean(np.abs(xa) > 3.0))
    tail_b = float(np.mean(np.abs(xb) > 3.0))

    warnings = []
    if ks_p < 0.05 and (wasserstein_sigma or 0) < _SIMILAR_SIGMA:
        warnings.append(
            f"KS rejects equality (p={ks_p:.3g}) while the Wasserstein distance is only "
            f"{wasserstein_sigma:.3f} sigma. On {min(xa.size, xb.size)} observations KS will "
            f"reject differences far too small to trade on; the effect size is the number to "
            f"read."
        )
    if ks_p >= 0.05 and (wasserstein_sigma or 0) >= _DIFFERENT_SIGMA:
        warnings.append(
            "KS does not reject and the effect size is large, which means the sample is too "
            "small for the test rather than the distributions being alike."
        )
    if tail_b > tail_a * 1.5 and tail_a > 0:
        warnings.append(
            f"the secondary series has {tail_b:.2%} of observations beyond three sigma against "
            f"{tail_a:.2%} for the primary. A stop sized on the primary will be hit far more "
            f"often on the secondary than the volatility ratio suggests."
        )
    if not standardise:
        warnings.append(
            "run without standardising, so most of any difference reported here is the "
            "difference in volatility between the two instruments rather than in the shape of "
            "their distributions."
        )

    detail = {
        "n_primary": int(xa.size),
        "n_secondary": int(xb.size),
        "standardised": standardise,
        "sigma_primary": sigma_a,
        "sigma_secondary": sigma_b,
        "ks_statistic": ks_statistic,
        "ks_p_value": ks_p,
        "wasserstein": wasserstein,
        "wasserstein_in_sigma": wasserstein_sigma,
        "verdict": verdict,
        "quantile_deltas": deltas,
        "worst_quantile": worst["quantile"],
        "worst_delta": worst["delta"],
        "tail_fraction_beyond_3_sigma": {"primary": tail_a, "secondary": tail_b},
        "thresholds_in_sigma": {"similar": _SIMILAR_SIGMA, "different": _DIFFERENT_SIGMA},
        "warnings": warnings,
        "instruments": {
            "primary": payload.get("instrument_id"),
            "secondary": payload.get("secondary_instrument_id"),
            "timeframe": payload.get("timeframe"),
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(detail, n=int(min(xa.size, xb.size)))
