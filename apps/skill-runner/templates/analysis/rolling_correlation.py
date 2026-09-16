"""How correlated are these two, and is that number stable enough to rely on.

```json
{
  "name": "rolling_correlation",
  "version": 1,
  "namespace": "analysis",
  "description": "Rolling correlation of two instruments with a block-bootstrap confidence band and a Marchenko-Pastur noise floor. Use it before assuming a hedge holds, or that two strategies are diversifying.",
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
      "window": {"type": "integer", "minimum": 20, "maximum": 2000, "default": 168}
    }
  },
  "summary_template": "full-sample correlation {correlation} (95% band {ci_low} to {ci_high}); rolling over {window} bars it ranges {rolling_min} to {rolling_max}; noise floor {noise_floor}; {stability_verdict}",
  "detail_fields": [
    ["correlation", ""],
    ["ci_low", ""],
    ["ci_high", ""],
    ["window", "bars"],
    ["rolling_min", ""],
    ["rolling_max", ""],
    ["noise_floor", ""],
    ["stability_verdict", ""]
  ],
  "wall_seconds": 120,
  "series": [
    {"role": "primary"},
    {"role": "secondary", "instrument_arg": "secondary_instrument_id"}
  ]
}
```

The point estimate is the least useful number here
---------------------------------------------------
A full-sample correlation of 0.6 that ranged from -0.1 to 0.9 over the window and
one that sat between 0.55 and 0.65 are the same number and completely different
facts. Only the second supports a hedge ratio. So the rolling range and the
stability verdict are what the summary carries, and the point estimate is there
to be qualified rather than quoted.

The band is a block bootstrap, not Fisher's z
----------------------------------------------
Fisher's transform gives a clean analytic interval and assumes i.i.d. bivariate
normal data. Returns are neither, and volatility clustering makes the true
interval materially wider than the analytic one — which is the direction that
matters, because it is the direction that turns "significantly correlated" into
"not measurably correlated".

The noise floor
---------------
Marchenko-Pastur gives the largest correlation two *independent* series of this
length would produce by chance: for a T-observation sample of N series the
eigenvalue edge is `(1 + sqrt(N/T))^2`, and for a pair the implied correlation
floor is `sqrt(N/T)` to first order. A measured correlation inside that floor is
not a small correlation, it is no correlation, and it should not be used to size
anything. On a pair the floor is small; the same arithmetic is what stops a
50-strategy covariance matrix estimated on 200 days from being almost entirely
noise, which is where it earns its place in the catalogue.
"""


def _rolling_correlation(a, b, window):
    n = a.size
    if n < window + 1:
        return np.empty(0)
    out = np.empty(n - window + 1)
    for i in range(out.size):
        x = a[i : i + window]
        y = b[i : i + window]
        sx, sy = x.std(ddof=1), y.std(ddof=1)
        out[i] = float(((x - x.mean()) * (y - y.mean())).mean() / (sx * sy)) if sx > 0 and sy > 0 else np.nan
    return out


def run(payload: dict) -> dict:
    a = load(payload, "primary")
    b = load(payload, "secondary")

    # Inner join, never a forward fill. Filling one series onto the other's
    # index correlates each series with its own last value, which inflates the
    # measured correlation toward whichever series is sparser.
    joined = a[["close"]].join(b[["close"]], how="inner", lsuffix="_a", rsuffix="_b").dropna()
    if len(joined) < 200:
        return refuse(
            f"only {len(joined)} bars are common to both series; below 200 the confidence band "
            f"is wider than the range the correlation can take",
            n_common=int(len(joined)),
        )

    pa = pd.to_numeric(joined["close_a"], errors="coerce").to_numpy(dtype=float)
    pb = pd.to_numeric(joined["close_b"], errors="coerce").to_numpy(dtype=float)
    keep = np.isfinite(pa) & np.isfinite(pb) & (pa > 0) & (pb > 0)
    ra = np.diff(np.log(pa[keep]))
    rb = np.diff(np.log(pb[keep]))
    n = int(ra.size)

    window = int(payload.get("window") or 168)
    window = max(20, min(window, n // 3))
    generator = rng(payload)

    correlation = float(np.corrcoef(ra, rb)[0, 1])

    # Block bootstrap of the *pair*: the two series are resampled with the same
    # block indices, so their contemporaneous relationship is preserved and only
    # the sampling error is resampled. Resampling them independently would
    # bootstrap the null rather than the estimate.
    block = max(optimal_block_length(ra), optimal_block_length(rb))
    draws = 2000
    block = int(min(max(1, block), n))
    per = int(math.ceil(n / block))
    starts = generator.integers(0, n - block + 1, size=(draws, per))
    offsets = np.arange(block)
    index = (starts[:, :, None] + offsets[None, None, :]).reshape(draws, per * block)[:, :n]
    sample_a = ra[index]
    sample_b = rb[index]
    centred_a = sample_a - sample_a.mean(axis=1, keepdims=True)
    centred_b = sample_b - sample_b.mean(axis=1, keepdims=True)
    numerator = (centred_a * centred_b).sum(axis=1)
    denominator = np.sqrt((centred_a**2).sum(axis=1) * (centred_b**2).sum(axis=1))
    with np.errstate(divide="ignore", invalid="ignore"):
        boot = np.where(denominator > 0, numerator / denominator, np.nan)
    boot = boot[np.isfinite(boot)]
    ci_low = float(np.percentile(boot, 2.5)) if boot.size else None
    ci_high = float(np.percentile(boot, 97.5)) if boot.size else None

    rolling = _rolling_correlation(ra, rb, window)
    rolling = rolling[np.isfinite(rolling)]
    rolling_min = float(rolling.min()) if rolling.size else None
    rolling_max = float(rolling.max()) if rolling.size else None
    rolling_sd = float(rolling.std(ddof=1)) if rolling.size > 1 else None

    # Marchenko-Pastur: two series, T observations.
    noise_floor = math.sqrt(2.0 / n)

    if rolling_max is not None and rolling_min is not None:
        spread = rolling_max - rolling_min
        sign_flips = bool(rolling_min < 0 < rolling_max)
        if sign_flips:
            stability_verdict = (
                "the rolling correlation changes sign, so there is no stable relationship to "
                "hedge with"
            )
        elif spread > 0.5:
            stability_verdict = (
                f"the rolling correlation spans {spread:.2f}, which is too wide for a fixed "
                f"hedge ratio"
            )
        elif spread > 0.25:
            stability_verdict = "moderately stable; re-estimate the hedge ratio periodically"
        else:
            stability_verdict = "stable over this window"
    else:
        sign_flips = False
        stability_verdict = "could not be assessed"

    warnings = []
    if abs(correlation) < noise_floor:
        warnings.append(
            f"the correlation of {correlation:.3f} is inside the Marchenko-Pastur noise floor "
            f"of {noise_floor:.3f} for {n} observations. That is not a weak relationship, it is "
            f"no measurable relationship, and it must not be used to size anything."
        )
    if ci_low is not None and ci_high is not None and ci_low < 0 < ci_high:
        warnings.append(
            "the 95% bootstrap band spans zero, so the sign of this correlation is not "
            "established."
        )
    if sign_flips:
        warnings.append(
            "the rolling correlation changes sign within the window. A portfolio built on the "
            "full-sample number is diversified in one regime and concentrated in the other, "
            "and the concentrated one is the drawdown."
        )

    detail = {
        "n_common_bars": n,
        "correlation": correlation,
        "ci_low": ci_low,
        "ci_high": ci_high,
        "bootstrap": {"draws": int(boot.size), "block_bars": int(block), "method": "paired moving block"},
        "window": window,
        "rolling_min": rolling_min,
        "rolling_max": rolling_max,
        "rolling_sd": rolling_sd,
        "rolling_mean": float(rolling.mean()) if rolling.size else None,
        "noise_floor": noise_floor,
        "noise_floor_note": "Marchenko-Pastur edge for two series over this many observations",
        "stability_verdict": stability_verdict,
        "warnings": warnings,
        "instruments": {
            "primary": payload.get("instrument_id"),
            "secondary": payload.get("secondary_instrument_id"),
            "timeframe": payload.get("timeframe"),
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(detail, n=n)
