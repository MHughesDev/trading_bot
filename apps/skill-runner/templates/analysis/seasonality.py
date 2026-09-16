"""Does the hour or the weekday matter, after paying for having looked at all of them.

```json
{
  "name": "seasonality",
  "version": 1,
  "namespace": "analysis",
  "description": "Mean return by hour of day and day of week, with Benjamini-Hochberg and Romano-Wolf corrections applied to every bucket. Survivors are reported with their adjusted p-values and never without.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "timeframe", "start", "end", "seed"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "seed": {"type": "integer"},
      "alpha": {"type": "number", "minimum": 0.001, "maximum": 0.2, "default": 0.05}
    }
  },
  "summary_template": "{n_buckets} buckets tested; {n_survivors} survive Romano-Wolf at the requested level ({survivors}); the raw scan would have reported {n_raw_significant}",
  "detail_fields": [
    ["n_buckets", "buckets"],
    ["n_survivors", ""],
    ["survivors", ""],
    ["n_raw_significant", ""]
  ],
  "wall_seconds": 120,
  "series": [{"role": "primary"}]
}
```

The correction is the tool
--------------------------
Testing 24 hours and 7 weekdays is 31 hypotheses. At a 5% level, roughly 1.5 of
them come back "significant" on pure noise, every time, forever. Every published
hour-of-day effect has to clear that bar and most do not.

Two corrections, both reported:

* **Benjamini-Hochberg** controls the false discovery rate and assumes the tests
  are independent or positively dependent. Hourly buckets of one price series
  are neither, quite.
* **Romano-Wolf** controls the family-wise error rate and takes the dependence
  from a block bootstrap of the data itself rather than assuming a structure.
  It is the stricter of the two and it is what the summary reports.

`n_raw_significant` is in the summary on purpose: the gap between it and
`n_survivors` is the size of the multiple-comparisons problem in this particular
sample, and showing it is more convincing than explaining it.

A bucket is never reported without its adjusted p-value. The raw p-value is in
`detail` for completeness and is not in the summary, because a raw p-value beside
a surviving bucket is an invitation to quote the smaller number.
"""


def _bucket_statistics(values, labels, n_labels):
    """Mean, HAC standard error and t-statistic per bucket."""
    out = []
    for k in range(n_labels):
        sample = values[labels == k]
        if sample.size < 10:
            out.append(None)
            continue
        mean = float(sample.mean())
        se = newey_west_se(sample)
        out.append(
            {
                "bucket": int(k),
                "n": int(sample.size),
                "mean": mean,
                "se": se,
                "t": float(mean / se) if se and math.isfinite(se) and se > 0 else float("nan"),
            }
        )
    return out


def run(payload: dict) -> dict:
    frame = load(payload, "primary")
    if not isinstance(frame.index, pd.DatetimeIndex):
        return refuse(
            "the materialised series has no timestamp index, so it cannot be bucketed by hour "
            "or weekday"
        )
    r = close_returns(frame)
    n = int(r.size)
    if n < 500:
        return refuse(
            "31 buckets over fewer than 500 bars leaves too few observations each for the "
            "correction to be doing anything but refusing everything",
            n_bars=n,
        )

    # `close_returns` drops the first bar and any non-positive close, so the
    # index is realigned rather than assumed to still match.
    index = frame.index[-n:]
    hours = index.hour.to_numpy()
    weekdays = index.dayofweek.to_numpy()

    alpha = float(payload.get("alpha") or 0.05)
    generator = rng(payload)

    families = [("hour", hours, 24), ("weekday", weekdays, 7)]
    names, stats, masks = [], [], []
    for family, labels, count in families:
        for entry in _bucket_statistics(r, labels, count):
            if entry is None:
                continue
            names.append(f"{family}_{entry['bucket']}")
            stats.append(entry)
            masks.append(labels == entry["bucket"])

    if len(names) < 2:
        return refuse("fewer than two buckets had enough observations to test", n_bars=n)

    t_values = np.array([s["t"] for s in stats], dtype=float)
    finite = np.isfinite(t_values)
    if not finite.all():
        t_values = np.where(finite, t_values, 0.0)

    # Raw two-sided p-values, for the comparison the summary makes.
    raw_p = np.array(
        [2.0 * st.norm.sf(abs(t)) if _HAVE_SCIPY else float("nan") for t in t_values]
    )
    n_raw_significant = int(np.sum(raw_p < alpha))

    bh_adjusted, bh_rejected = benjamini_hochberg(raw_p, alpha)

    # Romano-Wolf. The null distribution comes from a block bootstrap of the
    # whole return series, re-bucketed each draw — so the resampling keeps the
    # series' own dependence and destroys only the alignment between a return
    # and its hour, which is exactly the hypothesis.
    block = optimal_block_length(r)
    draws = 1000
    resampled = moving_block_bootstrap(r - r.mean(), draws, block, generator)
    #
    # One column per hypothesis, in the same order as `names` — the mask *is*
    # the hypothesis, so there is nothing to keep in step.
    boot_t = np.zeros((draws, len(names)))
    for column, mask in enumerate(masks):
        drawn = resampled[:, mask]
        if drawn.shape[1] < 2:
            continue
        mean = drawn.mean(axis=1)
        sd = drawn.std(axis=1, ddof=1)
        with np.errstate(divide="ignore", invalid="ignore"):
            t = np.where(sd > 0, mean / (sd / math.sqrt(drawn.shape[1])), 0.0)
        boot_t[:, column] = np.nan_to_num(t)

    rw_adjusted, rw_rejected = romano_wolf(t_values, boot_t, alpha)

    buckets = []
    for i, name in enumerate(names):
        buckets.append(
            {
                "name": name,
                "n": stats[i]["n"],
                "mean": stats[i]["mean"],
                "se": stats[i]["se"],
                "t": float(t_values[i]),
                "p_raw": float(raw_p[i]),
                "p_benjamini_hochberg": float(bh_adjusted[i]),
                "p_romano_wolf": float(rw_adjusted[i]),
                "survives": bool(rw_rejected[i]),
            }
        )
    buckets.sort(key=lambda b: b["p_romano_wolf"])

    survivors = [b["name"] for b in buckets if b["survives"]]

    warnings = []
    if n_raw_significant and not survivors:
        warnings.append(
            f"{n_raw_significant} buckets are significant before correction and none survive "
            f"it. That is the expected result on a series with no seasonality — at alpha="
            f"{alpha} and {len(names)} tests, roughly {alpha * len(names):.1f} false positives "
            f"are due. Reporting the uncorrected ones would be reporting the noise."
        )
    if survivors and len(survivors) == len(names):
        warnings.append(
            "every bucket survives, which is not a seasonality finding — it means the series "
            "has a non-zero mean and each bucket is detecting it."
        )

    detail = {
        "n_bars": n,
        "alpha": alpha,
        "n_buckets": len(names),
        "n_raw_significant": n_raw_significant,
        "n_survivors": len(survivors),
        "survivors": ", ".join(survivors) if survivors else "none",
        "buckets": buckets,
        "bootstrap": {"draws": draws, "block_bars": int(block)},
        "corrections": {
            "benjamini_hochberg": "false discovery rate, assumes independence or positive dependence",
            "romano_wolf": "family-wise error rate, dependence taken from the block bootstrap",
            "reported": "romano_wolf",
        },
        "warnings": warnings,
        "window": {
            "instrument_id": payload.get("instrument_id"),
            "timeframe": payload.get("timeframe"),
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(detail, n=n)
