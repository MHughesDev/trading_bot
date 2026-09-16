"""When has the market looked like this before, and what happened next.

```json
{
  "name": "find_analogs",
  "version": 1,
  "namespace": "analysis",
  "description": "Finds the historical windows most similar to the most recent one on an explicit, stated feature vector, and reports what followed each. Use it for a base rate, never for a forecast.",
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
      "window": {"type": "integer", "minimum": 10, "maximum": 1000, "default": 72},
      "horizon": {"type": "integer", "minimum": 1, "maximum": 500, "default": 24},
      "k": {"type": "integer", "minimum": 3, "maximum": 100, "default": 20}
    }
  },
  "summary_template": "the {k} closest analogs to the last {window} bars went {median_forward} (median) over the following {horizon}; {up_rate} of them rose, against a base rate of {base_rate}; nearest-analog distance {nearest_distance} against a random-pair median of {random_distance}",
  "detail_fields": [
    ["k", "analogs"],
    ["window", "bars"],
    ["horizon", "bars"],
    ["median_forward", ""],
    ["up_rate", "pct"],
    ["base_rate", "pct"],
    ["nearest_distance", ""],
    ["random_distance", ""]
  ],
  "wall_seconds": 120,
  "series": [{"role": "primary"}]
}
```

The feature vector is written down
-----------------------------------
Six numbers, and they are listed in `detail.features` on every call: cumulative
return over the window, realised volatility, the ratio of recent to trailing
volatility, skewness, the drawdown from the window's own high, and the fraction
of up bars. Each is scale-free, so 2019 and 2024 are comparable.

An embedding learned from the data would find better neighbours and nobody could
say what "similar" meant. This is the one place in the catalogue where an
explicit, worse feature set is the right choice: the output is a base rate that
a human is going to act on, and a base rate whose reference class cannot be
stated is not a base rate.

Why the random-pair distance is in the summary
-----------------------------------------------
With enough history, the *k* nearest neighbours of anything are close by
construction. `random_distance` is the median distance between two random windows
in the same series, and it is the only thing that says whether the analogs are
actually analogous. When `nearest_distance` is not much below it, the neighbours
are not similar to the present — they are merely the least dissimilar available,
and their forward returns are the unconditional distribution with extra steps.

Overlap and the base rate
--------------------------
Neighbouring windows overlap almost completely, so the *k* nearest would
otherwise be one episode counted *k* times. Analogs are therefore separated by at
least one window length. The base rate is the unconditional up-rate over the same
horizon, and it is beside the conditional one because "62% of analogs rose" means
nothing until you know that 58% of everything rose.
"""

_FEATURES = [
    "cumulative_return",
    "realised_volatility",
    "volatility_ratio",
    "skewness",
    "drawdown_from_window_high",
    "up_bar_fraction",
]


def _feature_matrix(r, close, window):
    """One row per window ending at bar i, in the order `_FEATURES` names."""
    n = r.size
    rows = n - window + 1
    if rows <= 0:
        return np.empty((0, len(_FEATURES)))
    out = np.empty((rows, len(_FEATURES)))
    short = max(3, window // 6)
    for i in range(rows):
        chunk = r[i : i + window]
        prices = close[i : i + window + 1]
        sd = float(np.std(chunk, ddof=1)) if window > 1 else 0.0
        recent = float(np.std(chunk[-short:], ddof=1)) if short > 1 else 0.0
        peak = float(np.max(prices))
        out[i] = (
            float(np.sum(chunk)),
            sd,
            recent / sd if sd > 0 else 1.0,
            float(np.mean(((chunk - chunk.mean()) / sd) ** 3)) if sd > 0 else 0.0,
            float(prices[-1] / peak - 1.0) if peak > 0 else 0.0,
            float(np.mean(chunk > 0)),
        )
    return out


def run(payload: dict) -> dict:
    frame = load(payload, "primary")
    close = pd.to_numeric(frame["close"], errors="coerce").to_numpy(dtype=float)
    close = close[np.isfinite(close) & (close > 0)]
    r = np.diff(np.log(close))
    n = int(r.size)

    window = int(payload.get("window") or 72)
    horizon = int(payload.get("horizon") or 24)
    k = int(payload.get("k") or 20)
    generator = rng(payload)

    if n < window * (k + 2) + horizon:
        return refuse(
            f"{k} non-overlapping analogs of {window} bars plus a {horizon}-bar forward window "
            f"needs more history than this window's {n} bars. Ask for fewer analogs, a shorter "
            f"window, or more history",
            n_bars=n,
        )

    features = _feature_matrix(r, close, window)
    # Standardised per column, so a feature's units do not decide the metric. An
    # unstandardised Euclidean distance over these six is dominated entirely by
    # whichever has the largest numerical range, which here is volatility.
    centre = features.mean(axis=0)
    scale = features.std(axis=0, ddof=1)
    scale[scale <= 0] = 1.0
    z = (features - centre) / scale

    query_index = features.shape[0] - 1
    query = z[query_index]

    # Candidates must have a full forward window, and must not overlap the query.
    last_usable = features.shape[0] - 1 - horizon
    distances = np.linalg.norm(z[: last_usable + 1] - query, axis=1)

    # Greedy non-overlapping selection: nearest first, then skip anything within
    # one window of an already-chosen analog. Without this the k nearest are one
    # episode counted k times, and every statistic below is computed on a sample
    # of size one.
    order = np.argsort(distances)
    chosen = []
    for idx in order:
        if len(chosen) >= k:
            break
        if any(abs(int(idx) - c) < window for c in chosen):
            continue
        chosen.append(int(idx))
    if len(chosen) < 3:
        return refuse(
            "fewer than three non-overlapping analogs were available",
            n_bars=n,
            n_analogs=len(chosen),
        )

    analogs = []
    forward = []
    for idx in chosen:
        end = idx + window
        path = r[end : end + horizon]
        if path.size < horizon:
            continue
        total = float(np.sum(path))
        forward.append(total)
        analogs.append(
            {
                "index": idx,
                "distance": float(distances[idx]),
                "forward_return": float(math.exp(total) - 1.0),
                "features": {name: float(features[idx][j]) for j, name in enumerate(_FEATURES)},
            }
        )
    forward = np.array(forward)
    if forward.size < 3:
        return refuse("fewer than three analogs had a complete forward window")

    # The unconditional base rate over the same horizon, from non-overlapping
    # windows so it is comparable to the conditional one.
    starts = np.arange(0, n - horizon, horizon)
    base = np.array([float(np.sum(r[s : s + horizon])) for s in starts])

    # How close is close? The median distance between two random windows says
    # whether these neighbours are near or merely nearest.
    sample_size = min(2000, z.shape[0])
    pick_a = generator.integers(0, z.shape[0], size=sample_size)
    pick_b = generator.integers(0, z.shape[0], size=sample_size)
    random_distance = float(np.median(np.linalg.norm(z[pick_a] - z[pick_b], axis=1)))
    nearest = float(min(a["distance"] for a in analogs))

    up_rate = float(100.0 * np.mean(forward > 0))
    base_rate = float(100.0 * np.mean(base > 0))

    warnings = []
    if nearest > random_distance * 0.5:
        warnings.append(
            f"the nearest analog is {nearest:.2f} away against a random-pair median of "
            f"{random_distance:.2f}. These windows are not similar to the present; they are the "
            f"least dissimilar available, and their forward returns are the unconditional "
            f"distribution."
        )
    if abs(up_rate - base_rate) < 10.0:
        warnings.append(
            f"the analogs rose {up_rate:.0f}% of the time against a base rate of "
            f"{base_rate:.0f}%. That difference is not a signal; conditioning on this state "
            f"told you nothing."
        )
    warnings.append(
        "this is a base rate over a stated reference class, not a forecast. It has no p-value "
        "and no confidence interval, because the analogs were selected for similarity to the "
        "present and any test computed on them would be conditioned on that selection."
    )

    detail = {
        "n_bars": n,
        "window": window,
        "horizon": horizon,
        "k": len(analogs),
        "features": _FEATURES,
        "query_features": {name: float(features[query_index][j]) for j, name in enumerate(_FEATURES)},
        "analogs": analogs,
        "median_forward": float(math.exp(float(np.median(forward))) - 1.0),
        "mean_forward": float(math.exp(float(np.mean(forward))) - 1.0),
        "forward_p05": float(math.exp(float(np.percentile(forward, 5))) - 1.0),
        "forward_p95": float(math.exp(float(np.percentile(forward, 95))) - 1.0),
        "up_rate": up_rate,
        "base_rate": base_rate,
        "base_rate_n": int(base.size),
        "nearest_distance": nearest,
        "random_distance": random_distance,
        "warnings": warnings,
        "window_spec": {
            "instrument_id": payload.get("instrument_id"),
            "timeframe": payload.get("timeframe"),
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(detail, n=len(analogs))
