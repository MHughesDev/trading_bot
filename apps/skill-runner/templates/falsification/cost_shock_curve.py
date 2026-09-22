"""cost_shock_curve — how much worse can costs get before the edge is gone.

```json
{
  "name": "cost_shock_curve",
  "namespace": "falsification",
  "risk": "read",
  "description": "Re-prices a run's realised trades at multiples of its assumed cost and reports the multiple at which the edge reaches zero. The grid starts from the venue spread measured on this instrument, not from a textbook basis-point figure.",
  "input_schema": {
    "type": "object",
    "required": ["run_id", "seed"],
    "properties": {
      "run_id": {"type": "string"},
      "seed": {"type": "integer"},
      "multipliers": {"type": "array", "items": {"type": "number"}},
      "measured_cost_bps": {"type": "number"},
      "assumed_cost_bps": {"type": "number"}
    }
  },
  "output": {
    "summary_template": "{run_id}: assumed {assumed_cost_bps} bps per side against a measured {measured_cost_bps} bps ({cost_origin}). n={n} trades, {turnover} round trips. Break-even at {breakeven_multiplier}x assumed cost; safety margin against measured is {safety_margin}x. {verdict}",
    "detail_fields": [
      ["run_id", ""], ["n", "trades"], ["turnover", "round trips"],
      ["assumed_cost_bps", "bps"], ["measured_cost_bps", "bps"], ["cost_origin", ""],
      ["breakeven_multiplier", "x"], ["safety_margin", "x"], ["verdict", ""]
    ]
  },
  "evidence": ["run_id"],
  "limits": {"wall_seconds": 60}
}
```

The grid's origin is the point. A cost sweep anchored on a textbook 5 bps is a
sweep around a number nobody measured, and the multiplier it reports is a
multiple of a guess. This template anchors on `measured_cost_bps` — the
effective spread observed on this instrument, supplied by the harness from the
same estimator `estimate_liquidity_cost` uses — and reports `cost_origin` so the
reader can see which anchor the numbers came from. When no measurement is
available the origin says `assumed_only`, and the safety margin is withheld
rather than computed against a number that does not exist.

`safety_margin` is the break-even multiple restated against *measured* cost
rather than assumed cost. A strategy that breaks even at 3x its assumed 2 bps is
comfortable if the market charges 2 bps and already dead if it charges 7.
"""

import json
import sys

import numpy as np
import pandas as pd


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


def main():
    payload = json.loads(sys.stdin.read())
    seed = int(payload["seed"])
    np.random.seed(seed)
    multipliers = [float(m) for m in payload.get("multipliers", [1.0, 1.5, 2.0, 3.0, 5.0])]
    assumed_bps = payload.get("assumed_cost_bps")
    measured_bps = payload.get("measured_cost_bps")

    df = pd.read_parquet(payload["data_path"])
    col = next((c for c in ("return_pct", "pnl_pct", "return") if c in df.columns), None)
    if col is None:
        fail(
            "missing_column",
            "the materialised trade list has no per-trade return column.",
            "materialise the run's trade list, not its equity curve",
        )
    gross = pd.to_numeric(df[col], errors="coerce").astype(float).to_numpy()
    gross = gross[np.isfinite(gross)]
    if np.max(np.abs(gross)) > 1.5:
        gross = gross / 100.0
    n = int(len(gross))
    if n < 10:
        fail("too_few_trades", f"{n} trades cannot support a cost sweep.", "widen the run's window")

    if assumed_bps is None:
        cost_col = next((c for c in ("cost_bps", "fees_bps", "commission_bps") if c in df.columns), None)
        if cost_col is None:
            fail(
                "cost_unknown",
                "neither `assumed_cost_bps` nor a per-trade cost column is present.",
                "pass assumed_cost_bps, or materialise a trade list that carries its costs",
            )
        assumed_bps = float(np.nanmean(pd.to_numeric(df[cost_col], errors="coerce")))
    assumed_bps = float(assumed_bps)

    if measured_bps is None:
        cost_origin = "assumed_only — no measured spread was supplied"
    else:
        measured_bps = float(measured_bps)
        cost_origin = f"measured effective spread, {measured_bps} bps per side"

    # Each round trip pays the cost twice; the trade list is already net of the
    # assumed cost, so add it back before re-charging at each multiple.
    per_trip = 2.0 * assumed_bps / 1e4
    base_gross = gross + per_trip
    turnover = n

    curve = []
    for m in multipliers:
        net = base_gross - m * per_trip
        total = float(np.prod(1.0 + net) - 1.0)
        mean = float(np.mean(net))
        se = float(np.std(net, ddof=1)) / np.sqrt(n) if n > 1 else float("nan")
        curve.append(
            {
                "multiplier": m,
                "cost_bps_per_side": round(m * assumed_bps, 3),
                "total_return_pct": round(100.0 * total, 3),
                "mean_trade_bps": round(1e4 * mean, 3),
                "mean_trade_se_bps": round(1e4 * se, 3),
                "profitable": bool(mean > 0),
            }
        )

    # Break-even in closed form: the mean gross edge divided by the round-trip
    # cost. Interpolating the sampled grid would report whichever multiple
    # happened to be asked for.
    mean_gross = float(np.mean(base_gross))
    breakeven = mean_gross / per_trip if per_trip > 0 else float("inf")
    breakeven = max(0.0, breakeven)

    if measured_bps is None:
        safety = None
        safety_text = "withheld (no measured spread)"
    else:
        implied = measured_bps / assumed_bps if assumed_bps > 0 else float("inf")
        safety = breakeven / implied if implied > 0 else float("inf")
        safety_text = round(safety, 2)

    if safety is None:
        verdict = "inconclusive: measure the spread before treating this edge as costed."
    elif safety < 1.0:
        verdict = "REJECT: at the measured spread this strategy already loses money."
    elif safety < 2.0:
        verdict = "REJECT under the pre-capital rule: less than 2x margin over measured cost."
    else:
        verdict = "passes the 2x cost margin."

    detail = {
        "run_id": payload["run_id"],
        "n": n,
        "turnover": turnover,
        "assumed_cost_bps": round(assumed_bps, 3),
        "measured_cost_bps": None if measured_bps is None else round(measured_bps, 3),
        "cost_origin": cost_origin,
        "curve": curve,
        "breakeven_multiplier": round(breakeven, 2),
        "safety_margin": safety_text,
        "mean_gross_edge_bps": round(1e4 * mean_gross, 3),
        "round_trip_cost_bps": round(1e4 * per_trip, 3),
        "verdict": verdict,
        "provenance": provenance(),
        "seed": seed,
    }
    print(json.dumps({"summary": None, "n": n, "detail": detail}))


if __name__ == "__main__":
    main()
