"""What happens after X, against what happens after nothing in particular.

```json
{
  "name": "event_study",
  "version": 1,
  "namespace": "analysis",
  "description": "Forward return paths after a named event, compared against volatility-matched random entries with block-bootstrap bands. Use it to check whether a pattern is worth building a strategy on before spending a trial.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "timeframe", "start", "end", "event", "seed"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "seed": {"type": "integer"},
      "event": {
        "type": "string",
        "enum": ["large_up_move", "large_down_move", "volatility_spike", "volatility_collapse", "new_high", "new_low"],
        "description": "which bars count as events; the size that counts as large is fixed by the platform"
      },
      "horizon": {"type": "integer", "minimum": 1, "maximum": 500, "default": 24,
                  "description": "bars of forward path to measure"},
      "lookback": {"type": "integer", "minimum": 20, "maximum": 2000, "default": 168,
                   "description": "trailing bars defining `sigma` and the volatility match"}
    }
  },
  "summary_template": "{n_events} events; mean forward return over {horizon} bars is {mean_forward} against {mean_control} for matched controls, a difference of {excess} ({verdict}, p={p_value})",
  "detail_fields": [
    ["n_events", "events"],
    ["horizon", "bars"],
    ["mean_forward", ""],
    ["mean_control", ""],
    ["excess", ""],
    ["verdict", ""],
    ["p_value", ""]
  ],
  "wall_seconds": 120,
  "series": [{"role": "primary"}]
}
```

The control is the whole tool
-----------------------------
"Price rose 2% in the day after a 3-sigma drop" is not a finding. Price rises
after most things, drops are clustered in high-volatility periods, and
high-volatility periods have wide forward distributions in both directions. A
comparison against zero measures the drift of the instrument plus the volatility
of the regime the events happened to live in.

So every event is matched to a random non-event entry **in the same volatility
decile**, drawn from the same series. The excess return is against that, and the
p-value comes from a block bootstrap of the matched difference rather than a
t-test — the events overlap, their forward windows overlap, and a t-test on
overlapping windows reports a standard error that is too small by a factor that
grows with the horizon.

What is deliberately not here
-----------------------------
**The event size is not an argument.** It is fixed at `EVENT_SIGMA` below, and
that is not an oversight about flexibility — sweeping it until the p-value drops
is exactly the search this platform counts trials to prevent, and a tool that
offered the knob as a convenience would be the easiest place in the catalogue to
do it by accident.

The first version of this file shipped the knob anyway, under a docstring saying
this. `inv23_no_governance_fields_in_tool_schemas` and
`spec_15_tool_surface_conformance` both refused it: INVARIANTS forbids a tool
schema exposing a threshold, and they were right about this one for the reason
written directly above them.

If a different size is genuinely wanted, that is a sweep over a registered
family, and it spends trials like any other.

Overlap
-------
Events closer together than `horizon` share forward bars. They are kept — throwing
them away would bias the sample toward isolated events, which are a different
population — and the block bootstrap's block length is set to at least `horizon`
so the resampling respects the dependence that the overlap creates.
"""

#: How large a move counts as an event, in trailing standard deviations.
#:
#: A platform constant rather than an argument. Three sigma on a fat-tailed
#: series is roughly the 99.5th percentile of |r| — rare enough to be an event
#: and common enough to leave a testable sample on a year of hourly bars.
EVENT_SIGMA = 3.0


def _trailing_sigma(r, lookback):
    """Standard deviation of the preceding `lookback` bars, for each bar.

    Causal: bar `i`'s sigma uses bars before `i` and never `i` itself. Using the
    bar's own return in its own threshold makes every large move automatically a
    large move relative to a window it inflated.
    """
    n = r.size
    sigma = np.full(n, np.nan)
    squared = r**2
    cumulative = np.concatenate([[0.0], np.cumsum(squared)])
    cumulative_mean = np.concatenate([[0.0], np.cumsum(r)])
    for i in range(lookback, n):
        s2 = (cumulative[i] - cumulative[i - lookback]) / lookback
        m = (cumulative_mean[i] - cumulative_mean[i - lookback]) / lookback
        v = s2 - m * m
        sigma[i] = math.sqrt(v) if v > 0 else np.nan
    return sigma


def _event_mask(kind, r, frame, sigma, threshold, lookback):
    n = r.size
    mask = np.zeros(n, dtype=bool)
    with np.errstate(invalid="ignore"):
        if kind == "large_up_move":
            mask = r > threshold * sigma
        elif kind == "large_down_move":
            mask = r < -threshold * sigma
        elif kind in ("volatility_spike", "volatility_collapse"):
            # The realised volatility of the last few bars against the trailing
            # window, so a "spike" is a change in regime rather than one bar.
            short = max(3, lookback // 12)
            recent = np.full(n, np.nan)
            squared = r**2
            cumulative = np.concatenate([[0.0], np.cumsum(squared)])
            for i in range(short, n):
                recent[i] = math.sqrt((cumulative[i + 1] - cumulative[i + 1 - short]) / short)
            ratio = recent / sigma
            mask = ratio > threshold if kind == "volatility_spike" else ratio < 1.0 / threshold
        elif kind in ("new_high", "new_low"):
            close = pd.to_numeric(frame["close"], errors="coerce").to_numpy(dtype=float)
            close = close[np.isfinite(close) & (close > 0)]
            # `close` is one longer than `r`; align to the return index.
            close = close[1:]
            for i in range(lookback, min(n, close.size)):
                window = close[i - lookback : i]
                if kind == "new_high":
                    mask[i] = close[i] > window.max()
                else:
                    mask[i] = close[i] < window.min()
    mask = np.asarray(mask, dtype=bool)
    mask[:lookback] = False
    return mask


def run(payload: dict) -> dict:
    frame = load(payload, "primary")
    r = close_returns(frame)
    n = int(r.size)
    generator = rng(payload)

    horizon = int(payload.get("horizon") or 24)
    lookback = int(payload.get("lookback") or 168)
    threshold = EVENT_SIGMA
    kind = payload.get("event") or "large_down_move"

    if n < lookback + horizon * 10:
        return refuse(
            f"an event study needs room for the lookback and many horizons; this window has "
            f"{n} bars against a lookback of {lookback} and a horizon of {horizon}",
            n_bars=n,
        )

    sigma = _trailing_sigma(r, lookback)
    mask = _event_mask(kind, r, frame, sigma, threshold, lookback)
    # An event in the last `horizon` bars has no forward path to measure.
    mask[n - horizon :] = False
    events = np.flatnonzero(mask)

    if events.size < 10:
        return refuse(
            f"only {events.size} `{kind}` events at {threshold} sigma over this window. Below "
            f"ten there is nothing to compare, so widen the window or pick an event class that "
            f"fires more often. The event size is fixed by the platform and is not an argument, "
            f"because lowering it until a result appears is a search",
            n_events=int(events.size),
            event=kind,
            event_sigma=threshold,
        )

    # Cumulative forward return from each entry.
    def forward_paths(indices):
        return np.array(
            [np.cumsum(r[i + 1 : i + 1 + horizon]) for i in indices], dtype=float
        )

    event_paths = forward_paths(events)

    # Volatility-matched controls. Each event is matched to a non-event bar in
    # the same volatility decile; without the match the "excess" is mostly the
    # difference between a violent regime and a calm one.
    eligible = np.flatnonzero(
        np.isfinite(sigma) & ~mask & (np.arange(n) >= lookback) & (np.arange(n) < n - horizon)
    )
    if eligible.size < events.size:
        return refuse(
            "there are fewer eligible control bars than events, which means the event "
            "definition covers most of the sample and there is nothing to compare against",
            n_events=int(events.size),
            n_eligible=int(eligible.size),
        )

    deciles = np.nanpercentile(sigma[eligible], np.arange(10, 100, 10))
    control_index = []
    unmatched = 0
    for i in events:
        bucket = int(np.searchsorted(deciles, sigma[i]))
        same = eligible[np.searchsorted(deciles, sigma[eligible]) == bucket]
        if same.size == 0:
            unmatched += 1
            same = eligible
        control_index.append(int(generator.choice(same)))
    control_paths = forward_paths(np.array(control_index))

    mean_forward = event_paths.mean(axis=0)
    mean_control = control_paths.mean(axis=0)
    difference = event_paths[:, -1] - control_paths[:, -1]
    excess = float(difference.mean())

    # Block bootstrap of the matched difference. The block is at least the
    # horizon, because two events within a horizon of each other share forward
    # bars and resampling them independently would pretend they were independent.
    block = max(horizon, optimal_block_length(difference))
    draws = moving_block_bootstrap(difference - difference.mean(), 2000, block, generator)
    null_means = draws.mean(axis=1)
    p_value = float((np.sum(np.abs(null_means) >= abs(excess)) + 1) / (null_means.size + 1))

    band_low = np.percentile(event_paths, 5, axis=0)
    band_high = np.percentile(event_paths, 95, axis=0)

    verdict = (
        "the forward path differs from matched controls"
        if p_value < 0.05
        else "indistinguishable from matched controls"
    )

    warnings = []
    if unmatched:
        warnings.append(
            f"{unmatched} of {events.size} events had no control in their own volatility "
            f"decile and were matched against the whole eligible set; the comparison is weaker "
            f"for those."
        )
    # Overlap, stated rather than silently corrected.
    gaps = np.diff(events)
    overlapping = int(np.sum(gaps < horizon))
    if overlapping:
        warnings.append(
            f"{overlapping} of {events.size - 1} consecutive event pairs are closer together "
            f"than the {horizon}-bar horizon, so their forward windows overlap. The block "
            f"bootstrap accounts for this; a t-test on these paths would not, and would report "
            f"a standard error far too small."
        )
    if p_value < 0.05 and excess * mean_forward[-1] < 0:
        warnings.append(
            "the excess and the raw forward return have opposite signs: the effect is entirely "
            "in the control, meaning this event marks a regime rather than a move."
        )

    detail = {
        "event": kind,
        "event_sigma": threshold,
        "lookback_bars": lookback,
        "horizon": horizon,
        "n_bars": n,
        "n_events": int(events.size),
        "n_overlapping_pairs": overlapping,
        "mean_forward": float(mean_forward[-1]),
        "mean_control": float(mean_control[-1]),
        "excess": excess,
        "p_value": p_value,
        "verdict": verdict,
        "bootstrap": {"draws": int(null_means.size), "block_bars": int(block)},
        "path": {
            "bars": list(range(1, horizon + 1)),
            "event_mean": mean_forward.tolist(),
            "control_mean": mean_control.tolist(),
            "event_p05": band_low.tolist(),
            "event_p95": band_high.tolist(),
        },
        "hit_rate": float(np.mean(event_paths[:, -1] > control_paths[:, -1])),
        "warnings": warnings,
        "window": {
            "instrument_id": payload.get("instrument_id"),
            "timeframe": payload.get("timeframe"),
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(
        detail,
        n=int(events.size),
        se=float(np.std(difference, ddof=1) / math.sqrt(events.size)),
    )
