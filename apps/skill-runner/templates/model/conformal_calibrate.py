"""Prediction intervals that stay honest when the market changes.

```json
{
  "name": "conformal_calibrate",
  "version": 1,
  "namespace": "model",
  "description": "Adaptive conformal intervals (ACI) around a model version's forecast of the forward return, using only labels resolved at each decision, with realised coverage overall and by block and a drift warning. Never vanilla split-conformal.",
  "input_schema": {
    "type": "object",
    "required": ["model", "instrument_id", "timeframe", "start", "end", "horizon_bars"],
    "additionalProperties": false,
    "properties": {
      "model": {"type": "string", "description": "name@version; series must be filled"},
      "output": {"type": "string", "description": "scalar output; omit if only one"},
      "instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "horizon_bars": {"type": "integer", "minimum": 1, "maximum": 2000,
                       "description": "forecast horizon in bars"},
      "coverage": {"type": "number", "minimum": 0.5, "maximum": 0.99, "default": 0.9},
      "method": {"type": "string", "enum": ["aci"], "default": "aci",
                 "description": "EnbPI is not offered"}
    }
  },
  "summary_template": "{method} at target {target_coverage}: realised coverage {realized_coverage} over {n_intervals} intervals (blocks {coverage_by_block}); median half-width {median_half_width}; {drift_warning}",
  "detail_fields": [
    ["method", ""],
    ["target_coverage", ""],
    ["realized_coverage", ""],
    ["n_intervals", "intervals"],
    ["coverage_by_block", ""],
    ["median_half_width", ""],
    ["drift_warning", ""]
  ],
  "wall_seconds": 120,
  "series": [{"role": "predictions", "kind": "predictions"}]
}
```

Why ACI, and why labels arrive late
-----------------------------------
Split conformal prediction guarantees coverage when the calibration and test
points are exchangeable. Returns are not: volatility clusters and regimes
switch, so a quantile fitted on a calm calibration set under-covers in a storm
and nobody is told. Adaptive conformal inference (Gibbs & Candes, 2021) tracks
the miscoverage level online: after each resolved label,
`alpha <- alpha + gamma * (alpha_target - miss)`, so a run of misses widens the
next intervals and a run of hits narrows them. Its guarantee is long-run
average coverage with no distributional assumption, which is the one this
tool can honestly make. `gamma` is a platform constant (0.005): a caller who
could pick it could pick the value at which the reported coverage looks best.

**A label is not available at the decision.** The label of a decision at `t`
is the return over the next `horizon_bars` bars, so it resolves at
`t + horizon_bars`. The interval at `t` is built, and `alpha` updated, only from
decisions whose label had resolved by `t`. Updating on a label that had not yet
happened would make the realised coverage a measurement of the future leaking
into the interval.

The nonconformity score is `|y - y_hat|` and the interval is
`y_hat +- q`, with `q` the finite-sample-corrected `1 - alpha` quantile of the
most recent 500 resolved scores. The first 50 resolved scores are warm-up:
those decisions get no interval and are counted, not scored.

Coverage by block
-----------------
Realised coverage is reported over five equal blocks of decisions as well as
overall. An average at target can hide a first half that over-covers and a
second half that collapses; the block series is what shows it. `drift_warning`
fires when overall coverage is more than 3 points from target, or any block is
more than 7 points from it, or the last block is more than 5 points below the
first. The thresholds are stated here and are not arguments.

Overlapping labels (`horizon_bars` > 1) are serially dependent, which is
exactly the situation ACI's assumption-free guarantee is for, and is the reason
exchangeability-based split conformal is refused.
"""

_GAMMA = 0.005
_WARMUP = 50
_WINDOW = 500


def _bar_seconds(timeframe: str) -> float:
    unit = timeframe[-1:].lower()
    try:
        size = int(timeframe[:-1])
    except ValueError:
        return float("nan")
    return float({"s": 1, "m": 60, "h": 3600, "d": 86400, "w": 604800}.get(unit, float("nan"))) * size


def run(payload: dict) -> dict:
    frame = pd.read_parquet((payload.get("data") or {}).get("predictions"))
    horizon = int(payload["horizon_bars"])
    target = float(payload.get("coverage") if payload.get("coverage") is not None else 0.9)
    col = f"y_h{horizon}"
    if col not in frame.columns or "y_hat" not in frame.columns:
        return refuse(
            f"the materialised predictions have no `{col}` label column",
            columns=[str(c) for c in frame.columns],
        )
    bar_s = _bar_seconds(str(payload.get("timeframe") or ""))
    if not np.isfinite(bar_s):
        return refuse("the timeframe is not one of s/m/h/d/w", timeframe=payload.get("timeframe"))

    ts = epoch_seconds(pd.DatetimeIndex(pd.to_datetime(frame["ts"], utc=True)))
    y_hat = pd.to_numeric(frame["y_hat"], errors="coerce").to_numpy(dtype=float)
    y = pd.to_numeric(frame[col], errors="coerce").to_numpy(dtype=float)
    ok = np.isfinite(y_hat) & np.isfinite(y)
    order = np.argsort(ts[ok], kind="stable")
    ts, y_hat, y = ts[ok][order], y_hat[ok][order], y[ok][order]
    n = int(ts.size)
    if n < _WARMUP + 100:
        return refuse(
            f"only {n} labelled decisions; the first {_WARMUP} are warm-up and a coverage "
            f"figure needs a few hundred more to mean anything",
            n_labelled=n,
        )

    resolve_at = ts + horizon * bar_s
    score = np.abs(y - y_hat)
    alpha = 1.0 - target
    resolved_idx = np.argsort(resolve_at, kind="stable")
    pointer = 0  # next label (by resolution time) not yet folded into alpha / scores
    folded_scores = []
    miss_flags = {}  # decision index -> miss, filled when its interval was built

    covered = []
    widths = []
    decision_rows = []
    warmup_skipped = 0
    for t in range(n):
        # Fold in every label that has resolved by this decision's time.
        while pointer < n and resolve_at[resolved_idx[pointer]] <= ts[t]:
            j = int(resolved_idx[pointer])
            folded_scores.append(float(score[j]))
            if j in miss_flags:
                alpha = alpha + _GAMMA * ((1.0 - target) - miss_flags[j])
            pointer += 1
        if len(folded_scores) < _WARMUP:
            warmup_skipped += 1
            continue
        pool = np.asarray(folded_scores[-_WINDOW:], dtype=float)
        level = (1.0 - alpha) * (1.0 + 1.0 / pool.size)
        if level >= 1.0:
            q = float("inf")
        elif level <= 0.0:
            q = 0.0
        else:
            q = float(np.quantile(pool, level))
        miss = 0.0 if score[t] <= q else 1.0
        miss_flags[t] = miss
        covered.append(1.0 - miss)
        widths.append(q)
        decision_rows.append(t)

    m = len(covered)
    if m < 100:
        return refuse(
            f"only {m} decisions got an interval after the warm-up; too few to report coverage",
            n_labelled=n,
            warmup_skipped=warmup_skipped,
        )
    cov = np.asarray(covered)
    finite_w = np.asarray([w for w in widths if np.isfinite(w)])
    blocks = [float(b.mean()) for b in np.array_split(cov, 5)]
    overall = float(cov.mean())
    warnings = []
    if abs(overall - target) > 0.03:
        warnings.append(f"overall coverage {overall:.3f} is more than 3 points from the {target:.2f} target")
    for i, b in enumerate(blocks):
        if abs(b - target) > 0.07:
            warnings.append(f"block {i + 1} of 5 covered {b:.3f}, more than 7 points from target")
    if blocks[0] - blocks[-1] > 0.05:
        warnings.append(
            f"coverage fell from {blocks[0]:.3f} in the first block to {blocks[-1]:.3f} in the last"
        )
    drift = "; ".join(warnings) if warnings else "none"

    last = decision_rows[-1]
    detail = {
        "method": "aci",
        "target_coverage": target,
        "realized_coverage": overall,
        "coverage_by_block": [round(b, 4) for b in blocks],
        "n_intervals": m,
        "warmup_decisions_without_interval": warmup_skipped,
        "n_labelled": n,
        "median_half_width": float(np.median(finite_w)) if finite_w.size else None,
        "infinite_intervals": int(m - finite_w.size),
        "latest_interval": {
            "ts": int(ts[last]),
            "lower": float(y_hat[last] - widths[-1]),
            "upper": float(y_hat[last] + widths[-1]),
        },
        "final_alpha": alpha,
        "drift_warning": drift,
        "drift": bool(warnings),
        "nonconformity": "absolute residual |y - y_hat|",
        "gamma": _GAMMA,
        "horizon_bars": horizon,
        "window": {
            "model": payload.get("model"),
            "instrument_id": payload.get("instrument_id"),
            "timeframe": payload.get("timeframe"),
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(detail, n=m)
