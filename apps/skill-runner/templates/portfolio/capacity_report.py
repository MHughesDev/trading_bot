"""What an order of a given size costs, and the size at which a declared edge is gone.

```json
{
  "name": "capacity_report",
  "version": 1,
  "namespace": "portfolio",
  "description": "Round-trip cost against order size under square-root impact, with volatility and volume measured from the bars, and the break-even size for a gross edge you declare. A pre-trial estimate: Gate 4 measures it from a run's trades.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "timeframe", "start", "end"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "gross_edge_bps": {"type": "number", "minimum": 0, "description": "the edge per round trip you expect before costs, in basis points; omitted, only the cost curve is returned"},
      "fee_bps": {"type": "number", "minimum": 0, "default": 10, "description": "per side; the default is a declared placeholder, not a venue schedule"},
      "half_spread_bps": {"type": "number", "minimum": 0, "default": 2, "description": "per side; a declared placeholder, not an estimate from these bars"},
      "impact_y": {"type": "number", "minimum": 0, "default": 0.7, "description": "the square-root-law coefficient; 0.7 is the spec's pre-calibration default"}
    }
  },
  "summary_template": "ADV {adv_usd} USD, daily vol {daily_vol_pct}%; round-trip cost {cost_at_1pct_bps} bps at 1% of ADV; break-even order size {break_even_usd} USD ({break_even_pct_adv}% of ADV) for a {gross_edge_bps} bps edge",
  "detail_fields": [
    ["adv_usd", "usd"],
    ["daily_vol_pct", "pct"],
    ["cost_at_1pct_bps", "bps"],
    ["gross_edge_bps", "bps"],
    ["break_even_usd", "usd"],
    ["break_even_pct_adv", "pct"],
    ["verdict", ""]
  ],
  "wall_seconds": 30,
  "series": [{"role": "primary"}]
}
```

The model
---------
Price impact of an order of notional `Q` in a market trading `V` a day, with daily
volatility `sigma`, is the **square-root law**:

    impact per side = Y * sigma * sqrt(Q / V)

(Almgren et al.; Toth et al.; confirmed on BTC futures.) Costs per side are fee plus
half-spread plus that impact, and a round trip pays them twice:

    round-trip cost = 2 * (fee + half_spread + impact(Q))

A strategy has capacity where its edge per round trip stops exceeding that. Solving
`gross = 2 * (fee + half_spread) + 2 * Y * sigma * sqrt(Q / V)` for `Q`:

    Q* = V * ((gross / 2 - fee - half_spread) / (Y * sigma)) ** 2

and `Q* = 0` when the edge does not even cover the fixed costs.

What is measured and what is declared
-------------------------------------
* **Measured from the bars:** `V` is the mean daily dollar volume (close times
  volume, summed per UTC day) and `sigma` the daily standard deviation of log returns
  from the same bars.
* **Declared, and stated as such in the output:** the fee, the half-spread and `Y`.
  The bars carry no quote data and EDGE is not built (OQ-15), so a half-spread
  *estimated* from them would be a number with false authority. The defaults are
  placeholders; pass the venue's schedule and, when it exists, a measured spread.
* **Declared by you:** the gross edge. This tool never invents one. Without it, the
  cost curve is still returned: the question "what edge does a 1% of ADV order need
  to clear" has an answer with no strategy at all.

A pre-trial estimate, not Gate 4
--------------------------------
Gate 4 (`backtest::capacity`) measures capacity from a Run's actual trades. This
answers an earlier question, before there are trades: *can an edge of this size
survive being traded at this size in this instrument at all?* It is the figure a
hypothesis registration wants, and it is replaced by Gate 4's measurement as soon as a
run exists.

Aggregate capacity
------------------
`Q*` is the size of **one order**. A strategy that trades repeatedly can carry more
total capital than one order's break-even, spread over time, and less if its orders
cluster. That is not modelled here and the output says so.
"""

SIZES_PCT_ADV = [0.01, 0.1, 0.5, 1.0, 2.0, 5.0, 10.0]


def run(payload: dict) -> dict:
    frame = load(payload, "primary")
    need = [c for c in ("close", "volume") if c not in frame.columns]
    if need:
        return refuse(f"the bars lack {need}")
    close = frame["close"].astype(float)
    volume = frame["volume"].astype(float)
    ok = (close > 0) & (volume >= 0) & np.isfinite(close) & np.isfinite(volume)
    close, volume = close[ok], volume[ok]
    if close.size < 48:
        return refuse(f"{close.size} usable bars is too few to measure a daily volume and volatility")

    idx = pd.DatetimeIndex(close.index)
    if idx.tz is None:
        idx = idx.tz_localize("UTC")
    dollars = pd.Series((close * volume).to_numpy(), index=idx)
    daily_dollars = dollars.groupby(idx.normalize()).sum()
    # A partial first or last day would understate ADV, so only whole days count when there are enough.
    if daily_dollars.size >= 5:
        daily_dollars = daily_dollars.iloc[1:-1]
    adv = float(daily_dollars.mean())
    daily_close = pd.Series(close.to_numpy(), index=idx).groupby(idx.normalize()).last()
    daily_ret = np.log(daily_close).diff().dropna().to_numpy()
    if adv <= 0 or daily_ret.size < 5:
        return refuse("average daily dollar volume is not positive, or there are under five daily returns")
    sigma = float(np.std(daily_ret, ddof=1))
    if not (sigma > 0 and math.isfinite(sigma)):
        return refuse("the daily volatility is zero or not finite, so impact is undefined")

    fee = float(payload.get("fee_bps", 10.0))
    half = float(payload.get("half_spread_bps", 2.0))
    y = float(payload.get("impact_y", 0.7))
    gross = payload.get("gross_edge_bps")
    gross = None if gross is None else float(gross)

    def impact_bps(q_usd: float) -> float:
        return y * sigma * math.sqrt(q_usd / adv) * 1e4

    def round_trip_bps(q_usd: float) -> float:
        return 2.0 * (fee + half + impact_bps(q_usd))

    curve = []
    for pct in SIZES_PCT_ADV:
        q = adv * pct / 100.0
        row = {
            "pct_adv": pct,
            "order_usd": q,
            "impact_bps": impact_bps(q),
            "round_trip_cost_bps": round_trip_bps(q),
        }
        if gross is not None:
            row["net_edge_bps"] = gross - round_trip_bps(q)
        curve.append(row)

    break_even = None
    verdict = "no gross edge was declared, so only the cost curve is returned"
    if gross is not None:
        fixed = 2.0 * (fee + half)
        if gross <= fixed:
            break_even = 0.0
            verdict = (
                f"the {gross:.1f} bps edge does not cover the fixed round-trip cost of {fixed:.1f} bps "
                f"(fees and half-spread), so no order size is profitable"
            )
        elif y * sigma <= 0:
            return refuse("impact_y is zero, so the break-even size is unbounded")
        else:
            break_even = adv * (((gross / 2.0 - fee - half) / (y * sigma * 1e4)) ** 2)
            verdict = (
                f"net expectancy reaches zero at an order of about {break_even:,.0f} USD; "
                f"below that it is positive, above it the order costs more than it earns"
            )

    detail = {
        "instrument_id": payload.get("instrument_id"),
        "adv_usd": adv,
        "daily_vol_pct": sigma * 100.0,
        "n_days": int(daily_dollars.size),
        "cost_at_1pct_bps": round_trip_bps(adv * 0.01),
        "gross_edge_bps": gross,
        "break_even_usd": break_even,
        "break_even_pct_adv": None if break_even is None else 100.0 * break_even / adv,
        "verdict": verdict,
        "curve": curve,
        "assumptions": {
            "fee_bps": fee,
            "fee_source": "declared" if "fee_bps" in payload else "placeholder default",
            "half_spread_bps": half,
            "spread_source": "declared" if "half_spread_bps" in payload else "placeholder default (no quotes; EDGE not built)",
            "impact_y": y,
            "impact_y_source": "declared" if "impact_y" in payload else "spec default before calibration",
            "volume_and_volatility": "measured from the bars",
        },
        "warnings": [
            "this is the size of ONE order; aggregate capacity over time is not modelled",
            "a pre-trial estimate from a declared edge; Gate 4 measures capacity from a run's trades",
        ],
    }
    return result(detail, n=int(close.size))
