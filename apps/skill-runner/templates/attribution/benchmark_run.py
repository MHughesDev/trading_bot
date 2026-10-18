"""What dumb rules earn on the same bars, under the same costs.

```json
{
  "name": "benchmark_run",
  "version": 1,
  "namespace": "attribution",
  "description": "Buy-and-hold, a seeded random-entry baseline matched to a trade count and holding period, and simple momentum on the same bars and costs. Read a strategy's result beside these, never alone.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "timeframe", "start", "end"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "n_trades": {"type": "integer", "minimum": 1, "maximum": 5000, "default": 20, "description": "the strategy's trade count, so the random baseline trades as often"},
      "hold_bars": {"type": "integer", "minimum": 1, "maximum": 5000, "default": 24, "description": "the strategy's holding period in bars"},
      "momentum_lookback": {"type": "integer", "minimum": 2, "maximum": 5000, "default": 24},
      "fee_bps": {"type": "number", "minimum": 0, "default": 10, "description": "per side; a declared placeholder"},
      "half_spread_bps": {"type": "number", "minimum": 0, "default": 2, "description": "per side; a declared placeholder"},
      "n_draws": {"type": "integer", "minimum": 100, "maximum": 5000, "default": 500},
      "seed": {"type": "integer"}
    }
  },
  "summary_template": "buy-and-hold {buy_hold_return_pct}%, random entry median {random_median_pct}% (95th {random_p95_pct}%), momentum {momentum_return_pct}%; to beat all three a candidate needs {hurdle_pct}%",
  "detail_fields": [
    ["buy_hold_return_pct", "pct"],
    ["random_median_pct", "pct"],
    ["random_p95_pct", "pct"],
    ["momentum_return_pct", "pct"],
    ["hurdle_pct", "pct"],
    ["momentum_trades", ""],
    ["n_bars", ""]
  ],
  "wall_seconds": 60,
  "series": [{"role": "primary"}]
}
```

Why a result needs these beside it
----------------------------------
A strategy that returned 40% on a window in which the instrument rose 60% did not
make money from skill. One that returned 5% when a random entry with the same trade
count, hold and costs has a 95th percentile of 8% has not shown it can tell a good
entry from a bad one. A momentum rule is the cheapest thing that *could* have worked.
None of these is hard to beat in principle; reporting a result without them is how a
mediocre one gets believed.

The three baselines
-------------------
* **Buy-and-hold:** one round trip of fees and half-spread, then the bar-to-bar return.
* **Random entry, matched:** `n_trades` long trades of `hold_bars` bars each, entered at
  random **non-overlapping** slots (the window is cut into slots of `hold_bars`, and
  `n_trades` of them are drawn without replacement), each paying a round trip of costs.
  The number is the *sum of trade returns* for one draw, reported as a distribution
  over `n_draws` seeded draws, never a point. A trade count the window cannot hold is a
  refusal rather than a silent shrink.
* **Momentum:** long when the trailing `momentum_lookback`-bar return is positive,
  flat otherwise, decided on the previous bar's close and applied to this bar's return
  (one-bar lag: it never trades on the bar it earns), paying costs on every change.

The hurdle
----------
`hurdle_pct` is the best of buy-and-hold, the random entry's 95th percentile and
momentum: a candidate has beaten all three only if it clears that. The costs are the
declared placeholders unless you pass the venue's, and every figure is **gross of
nothing the strategy itself pays**, because the baselines are charged the same.
"""

QUANTILES = [5, 25, 50, 75, 95]


def run(payload: dict) -> dict:
    frame = load(payload, "primary")
    if "close" not in frame.columns:
        return refuse("the bars lack `close`")
    close = frame["close"].astype(float).to_numpy()
    close = close[np.isfinite(close) & (close > 0)]
    n = int(close.size)
    n_trades = int(payload.get("n_trades") or 20)
    hold = int(payload.get("hold_bars") or 24)
    lookback = int(payload.get("momentum_lookback") or 24)
    draws = int(payload.get("n_draws") or 500)
    fee = float(payload.get("fee_bps", 10.0))
    half = float(payload.get("half_spread_bps", 2.0))
    side = (fee + half) / 1e4  # one side

    if n < max(hold * 2, lookback + 5, 50):
        return refuse(f"{n} usable bars is too few for a {hold}-bar hold and a {lookback}-bar momentum look-back")
    slots = (n - 1) // hold
    if n_trades > slots:
        return refuse(
            f"the window holds {slots} non-overlapping {hold}-bar trades and {n_trades} were asked for; "
            f"shorten the hold or the trade count rather than letting the baseline overlap itself",
            slots=slots,
        )

    ret = close[1:] / close[:-1] - 1.0  # bar t return, from close t-1 to close t

    # Buy and hold: pay one side to enter and one to exit.
    bh = float(close[-1] / close[0] - 1.0 - 2.0 * side)

    # Random entry, matched. Slot k covers bars [k*hold, k*hold + hold].
    gen = rng(payload)
    totals = np.empty(draws)
    slot_returns = close[hold::hold][:slots] / close[0 : slots * hold : hold] - 1.0 - 2.0 * side
    for d in range(draws):
        pick = gen.choice(slots, size=n_trades, replace=False)
        totals[d] = float(np.sum(slot_returns[pick]))
    qs = {q: float(np.percentile(totals, q)) for q in QUANTILES}

    # Momentum, decided on the previous close and applied to this bar. position[t] uses closes <= t-1.
    pos = np.zeros(ret.size)
    for t in range(ret.size):
        j = t  # ret[t] is the return of close[t+1]/close[t]; the decision uses close[t] and close[t-lookback]
        if j - lookback >= 0:
            pos[t] = 1.0 if close[j] / close[j - lookback] - 1.0 > 0 else 0.0
    # Costs are charged in the bar they are incurred, then compounded with the returns, so a
    # long-only rule can never lose more than everything. The last position is liquidated.
    change = np.abs(np.diff(np.concatenate([[0.0], pos])))
    net = pos * ret - change * side
    mom_total = float(np.prod(1.0 + net) * (1.0 - side * pos[-1]) - 1.0)
    trades = int(np.sum(np.diff(np.concatenate([[0.0], pos])) > 0))

    hurdle = max(bh, qs[95], mom_total)
    detail = {
        "instrument_id": payload.get("instrument_id"),
        "n_bars": n,
        "n_trades": n_trades,
        "hold_bars": hold,
        "momentum_lookback": lookback,
        "buy_hold_return_pct": bh * 100.0,
        "random_median_pct": qs[50] * 100.0,
        "random_p95_pct": qs[95] * 100.0,
        "random_quantiles_pct": {str(q): v * 100.0 for q, v in qs.items()},
        "momentum_return_pct": mom_total * 100.0,
        "momentum_trades": trades,
        "hurdle_pct": hurdle * 100.0,
        "n_draws": draws,
        "assumptions": {
            "fee_bps": fee,
            "half_spread_bps": half,
            "costs": "declared" if ("fee_bps" in payload or "half_spread_bps" in payload) else "placeholder defaults",
        },
        "warnings": [
            "the random baseline is a sum of non-overlapping long trades, not a compounded equity curve",
            "every baseline pays the same costs a strategy would; compare like with like",
        ],
    }
    return result(detail, n=n)
