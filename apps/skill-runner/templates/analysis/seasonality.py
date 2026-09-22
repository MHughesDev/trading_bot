"""seasonality — hour-of-day and day-of-week effects, corrected for multiplicity.

```json
{
  "name": "seasonality",
  "namespace": "analysis",
  "risk": "read",
  "description": "Mean return per hour-of-day or day-of-week with standard errors and multiple-testing-corrected p-values (Benjamini-Hochberg and Romano-Wolf). Use it before building any calendar rule; it reports no survivor without its adjusted p-value.",
  "input_schema": {
    "type": "object",
    "required": ["instrument", "timeframe", "start", "end", "seed"],
    "properties": {
      "instrument": {"type": "string"},
      "timeframe": {"type": "string", "enum": ["1m", "5m", "15m", "1h", "4h", "1d"]},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "seed": {"type": "integer"},
      "by": {"type": "string", "enum": ["hour", "weekday", "month"]},
      "n_bootstrap": {"type": "integer"},
      "block_length": {"type": "integer"}
    }
  },
  "output": {
    "summary_template": "{instrument} {timeframe} by {by}: {n_cells} cells over n={n} bars. Raw survivors at 5%: {raw_survivor_count}. After Benjamini-Hochberg: {bh_survivor_count}. After Romano-Wolf ({n_bootstrap} block resamples, block {block_length}): {rw_survivor_count} — {survivors}.",
    "detail_fields": [
      ["instrument", ""], ["timeframe", ""], ["by", ""], ["n", "bars"],
      ["n_cells", ""], ["raw_survivor_count", ""], ["bh_survivor_count", ""],
      ["rw_survivor_count", ""], ["survivors", ""],
      ["n_bootstrap", ""], ["block_length", "bars"]
    ]
  },
  "evidence": [],
  "limits": {"wall_seconds": 120}
}
```

The correction is not optional and there is no argument that switches it off.
Twenty-four hourly cells tested at 5% produce, on average, 1.2 "significant"
hours in pure noise; a weekday test produces 0.35. Every calendar anomaly that
has ever been backtested into existence was found by reporting the raw p-value
of the best cell, and the result is always presentable.

Two corrections because they answer different questions. Benjamini-Hochberg
controls the expected *share* of false discoveries among what is reported — the
right question when the output is a shortlist to investigate. Romano-Wolf
controls the probability of *any* false discovery and, unlike Bonferroni, it does
so while accounting for the dependence between cells, which is substantial here:
adjacent hours share the same overnight session. Romano-Wolf is the stricter of
the two and is the one to cite when a cell is about to become a trading rule.

The block bootstrap preserves the volatility clustering that makes an i.i.d.
resample far too optimistic. `block_length` defaults to the value
`describe_returns` derives for this series rather than a constant.
"""

import json
import math
import sys

import numpy as np
import pandas as pd

try:
    from scipy import stats as _scipy_stats
except ImportError:  # pragma: no cover
    _scipy_stats = None


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


def benjamini_hochberg(pvals):
    """Adjusted p-values controlling the false discovery rate."""
    p = np.asarray(pvals, dtype=float)
    m = len(p)
    order = np.argsort(p)
    ranked = p[order] * m / (np.arange(m) + 1.0)
    # Enforce monotonicity from the largest downwards.
    ranked = np.minimum.accumulate(ranked[::-1])[::-1]
    out = np.empty(m)
    out[order] = np.minimum(ranked, 1.0)
    return out


def romano_wolf(t_obs, boot_t):
    """Step-down max-T adjusted p-values (Romano & Wolf 2005).

    `boot_t` is (n_bootstrap, n_cells) of centred bootstrap t-statistics. At each
    step the largest remaining observed statistic is compared against the
    distribution of the maximum over the *remaining* cells, which is what makes
    this less conservative than Bonferroni while still controlling the
    family-wise error rate under dependence.
    """
    t_obs = np.abs(np.asarray(t_obs, dtype=float))
    boot = np.abs(np.asarray(boot_t, dtype=float))
    m = len(t_obs)
    order = np.argsort(-t_obs)
    remaining = list(order)
    adjusted = np.ones(m)
    running = 0.0
    for _ in range(m):
        j = remaining[0]
        max_boot = boot[:, remaining].max(axis=1)
        p = float((max_boot >= t_obs[j]).mean())
        running = max(running, p)  # monotone in the step-down order
        adjusted[j] = min(running, 1.0)
        remaining = remaining[1:]
    return adjusted


def block_indices(n, block, rng):
    """Circular moving-block bootstrap index set of length n."""
    starts = rng.integers(0, n, size=int(math.ceil(n / block)))
    idx = np.concatenate([(np.arange(s, s + block) % n) for s in starts])
    return idx[:n]


def main():
    payload = json.loads(sys.stdin.read())
    seed = int(payload["seed"])
    rng = np.random.default_rng(seed)
    by = payload.get("by", "hour")
    n_boot = int(payload.get("n_bootstrap", 2000))

    df = pd.read_parquet(payload["data_path"])
    if "ts" in df.columns:
        ts = pd.to_datetime(df["ts"], utc=True)
    elif isinstance(df.index, pd.DatetimeIndex):
        ts = df.index.tz_convert("UTC") if df.index.tz else df.index.tz_localize("UTC")
    else:
        fail(
            "missing_column",
            "the materialised bars carry no timestamp, so there is no calendar to test.",
            "request a series with a `ts` column",
        )
    close = pd.to_numeric(df["close"], errors="coerce").astype(float).to_numpy()
    r = np.diff(np.log(close))
    ts = pd.Series(ts).iloc[1:].reset_index(drop=True)
    ok = np.isfinite(r)
    r, ts = r[ok], ts[ok].reset_index(drop=True)
    n = int(len(r))
    if n < 200:
        fail("window_too_short", f"{n} returns cannot support a calendar test.", "widen start/end")

    if by == "hour":
        key = ts.dt.hour.to_numpy()
    elif by == "weekday":
        key = ts.dt.dayofweek.to_numpy()
    else:
        key = ts.dt.month.to_numpy()
    cells = sorted(set(int(k) for k in key))
    if len(cells) < 2:
        fail(
            "one_cell",
            f"every bar falls in the same `{by}` cell, so there is nothing to compare.",
            "use a finer timeframe, or group by something the window actually varies over",
        )

    block = int(payload.get("block_length", max(2, min(50, int(round(math.sqrt(n)))))))

    def stats_for(values):
        means, ses, ts_stat, counts = [], [], [], []
        for cell in cells:
            v = values[key == cell]
            k = len(v)
            counts.append(int(k))
            if k < 3:
                means.append(float("nan"))
                ses.append(float("nan"))
                ts_stat.append(0.0)
                continue
            m = float(np.mean(v))
            se = float(np.std(v, ddof=1)) / math.sqrt(k)
            means.append(m)
            ses.append(se)
            ts_stat.append(0.0 if se == 0 else m / se)
        return np.array(means), np.array(ses), np.array(ts_stat), counts

    means, ses, t_obs, counts = stats_for(r)

    if _scipy_stats is None:
        fail(
            "statistics_unavailable",
            "scipy is not installed, so no p-value can be computed.",
            "install scipy in the skill-runner image",
        )
    raw_p = np.array([2.0 * _scipy_stats.norm.sf(abs(t)) for t in t_obs])
    bh_p = benjamini_hochberg(raw_p)

    # Bootstrap under the null that no cell differs: recentre by the global mean
    # and resample in blocks, preserving the clustering.
    centred = r - float(np.mean(r))
    boot_t = np.empty((n_boot, len(cells)))
    for b in range(n_boot):
        idx = block_indices(n, block, rng)
        _, _, tb, _ = stats_for(centred[idx])
        boot_t[b] = tb
    rw_p = romano_wolf(t_obs, boot_t)

    rows = []
    for i, cell in enumerate(cells):
        rows.append(
            {
                "key": int(cell),
                "n": counts[i],
                "mean_bps": None if not np.isfinite(means[i]) else round(1e4 * means[i], 3),
                "se_bps": None if not np.isfinite(ses[i]) else round(1e4 * ses[i], 3),
                "t": round(float(t_obs[i]), 3),
                "p_raw": round(float(raw_p[i]), 5),
                "p_bh": round(float(bh_p[i]), 5),
                "p_romano_wolf": round(float(rw_p[i]), 5),
            }
        )

    survivors = [row["key"] for row in rows if row["p_romano_wolf"] < 0.05]
    detail = {
        "instrument": payload["instrument"],
        "timeframe": payload["timeframe"],
        "window": {"start": payload["start"], "end": payload["end"]},
        "by": by,
        "n": n,
        "n_cells": len(cells),
        "cells": rows,
        "raw_survivor_count": int(sum(1 for row in rows if row["p_raw"] < 0.05)),
        "bh_survivor_count": int(sum(1 for row in rows if row["p_bh"] < 0.05)),
        "rw_survivor_count": len(survivors),
        "survivors": ", ".join(str(s) for s in survivors) if survivors else "none",
        "correction": "benjamini_hochberg and romano_wolf; neither is optional",
        "n_bootstrap": n_boot,
        "block_length": block,
        "provenance": provenance(),
        "seed": seed,
    }
    print(json.dumps({"summary": None, "n": n, "detail": detail}))


if __name__ == "__main__":
    main()
