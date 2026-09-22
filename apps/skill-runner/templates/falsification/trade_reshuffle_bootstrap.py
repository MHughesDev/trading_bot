"""trade_reshuffle_bootstrap — path risk and estimation risk, never conflated.

```json
{
  "name": "trade_reshuffle_bootstrap",
  "namespace": "falsification",
  "risk": "read",
  "description": "Resamples a run's realised trade returns to bound either the path risk of a fixed outcome set (reshuffle) or the estimation uncertainty of its endpoint (bootstrap/block). Use it to size capital against drawdown, and read the mode before reading the numbers.",
  "input_schema": {
    "type": "object",
    "required": ["run_id", "mode", "seed"],
    "properties": {
      "run_id": {"type": "string"},
      "mode": {"type": "string", "enum": ["reshuffle", "bootstrap", "block"]},
      "seed": {"type": "integer"},
      "n_paths": {"type": "integer"},
      "block_length": {"type": "integer"},
      "capital_levels": {"type": "array", "items": {"type": "number"}}
    }
  },
  "output": {
    "summary_template": "{run_id} in {mode} mode ({mode_answers}) over {n_paths} paths from n={n} trades: max drawdown p50 {max_dd_p50}, p95 {max_dd_p95}, p99 {max_dd_p99}; longest time under water p95 {tuw_p95}. {endpoint_line} Ruin probability by capital: {ruin_line}. {mode_caveat}",
    "detail_fields": [
      ["run_id", ""], ["mode", ""], ["mode_answers", ""], ["n", "trades"],
      ["n_paths", ""], ["max_dd_p50", "%"], ["max_dd_p95", "%"], ["max_dd_p99", "%"],
      ["tuw_p95", "trades"], ["endpoint_line", ""], ["ruin_line", ""], ["mode_caveat", ""]
    ]
  },
  "evidence": ["run_id"],
  "limits": {"wall_seconds": 90}
}
```

The two modes answer different questions and conflating them is a documented
error, so this tool refuses to let the conflation happen quietly.

**`reshuffle`** permutes the realised trades. The multiset of outcomes is fixed,
so the endpoint is *identical on every path* — only the order, and therefore the
drawdown, varies. It answers: given exactly these trades, how unlucky could the
sequence have been? It says nothing about whether the edge is real, and its
dispersion is not a confidence interval for the return.

**`bootstrap`** resamples with replacement, so the endpoint varies. It answers:
if these trades are draws from some distribution, how uncertain is the mean?
**`block`** does the same while preserving runs of consecutive trades, which
matters whenever a strategy's wins cluster — and they usually do.

The refusal: in `reshuffle` mode `endpoint_dispersion` is absent and the result
is labelled `path_risk_of_a_fixed_outcome_set`. A reshuffle result cannot be
reported as a capital requirement or as a worst case, because the outcome set it
holds fixed is the one the search already selected.
"""

import json
import math
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


MODE_ANSWERS = {
    "reshuffle": "path risk of a fixed outcome set",
    "bootstrap": "estimation uncertainty of the endpoint",
    "block": "estimation uncertainty, preserving runs of consecutive trades",
}

MODE_CAVEAT = {
    "reshuffle": (
        "Reshuffle holds the outcome set fixed, so the endpoint is identical on every path. "
        "These percentiles are path risk, NOT a worst case and NOT a capital requirement: "
        "the outcome set they hold fixed is the one the search already selected."
    ),
    "bootstrap": "Independent resampling; if this strategy's wins cluster, use `block` instead.",
    "block": "Block resampling preserves runs of consecutive trades, which is what independent resampling loses.",
}


def path_stats(returns):
    """Equity, max drawdown and longest time under water for one ordering."""
    equity = np.cumprod(1.0 + returns)
    peak = np.maximum.accumulate(equity)
    dd = equity / peak - 1.0
    max_dd = float(np.min(dd))
    under = dd < 0
    longest, run = 0, 0
    for u in under:
        run = run + 1 if u else 0
        longest = max(longest, run)
    return float(equity[-1] - 1.0), max_dd, longest


def main():
    payload = json.loads(sys.stdin.read())
    seed = int(payload["seed"])
    rng = np.random.default_rng(seed)
    mode = payload["mode"]
    if mode not in MODE_ANSWERS:
        fail(
            "unknown_mode",
            f"`{mode}` is not one of reshuffle, bootstrap, block.",
            "pick reshuffle for path risk or bootstrap/block for estimation uncertainty",
        )
    n_paths = int(payload.get("n_paths", 5000))
    capital_levels = payload.get("capital_levels", [0.10, 0.20, 0.30, 0.50])

    df = pd.read_parquet(payload["data_path"])
    col = next((c for c in ("return_pct", "pnl_pct", "return") if c in df.columns), None)
    if col is None:
        fail(
            "missing_column",
            "the materialised trade list has no per-trade return column.",
            "materialise the run's trade list, not its equity curve",
        )
    returns = pd.to_numeric(df[col], errors="coerce").astype(float).to_numpy()
    returns = returns[np.isfinite(returns)]
    # A column in percent and a column in fractions look identical to a resampler
    # and differ by 100x in every drawdown it reports.
    if np.max(np.abs(returns)) > 1.5:
        returns = returns / 100.0
    n = int(len(returns))
    if n < 20:
        fail(
            "too_few_trades",
            f"{n} trades cannot support a resampling argument.",
            "widen the run's window, or accept that this run has no distribution to resample",
        )

    block = int(payload.get("block_length", max(2, min(n // 4, int(round(math.sqrt(n)))))))

    endpoints = np.empty(n_paths)
    max_dds = np.empty(n_paths)
    tuws = np.empty(n_paths)
    for i in range(n_paths):
        if mode == "reshuffle":
            draw = rng.permutation(returns)
        elif mode == "bootstrap":
            draw = returns[rng.integers(0, n, size=n)]
        else:
            starts = rng.integers(0, n, size=int(math.ceil(n / block)))
            idx = np.concatenate([(np.arange(s, s + block) % n) for s in starts])[:n]
            draw = returns[idx]
        endpoints[i], max_dds[i], tuws[i] = path_stats(draw)

    def pct(a, q):
        return round(100.0 * float(np.percentile(a, q)), 2)

    ruin = {
        f"{int(100 * c)}%": round(float(np.mean(max_dds <= -abs(c))), 4) for c in capital_levels
    }

    detail = {
        "run_id": payload["run_id"],
        "mode": mode,
        "mode_answers": MODE_ANSWERS[mode],
        "mode_caveat": MODE_CAVEAT[mode],
        "n": n,
        "n_paths": n_paths,
        "block_length": block if mode == "block" else None,
        "max_dd_pctiles": {
            "p50": pct(max_dds, 50),
            "p75": pct(max_dds, 75),
            "p95": pct(max_dds, 5),
            "p99": pct(max_dds, 1),
        },
        "max_dd_p50": pct(max_dds, 50),
        "max_dd_p95": pct(max_dds, 5),
        "max_dd_p99": pct(max_dds, 1),
        "tuw_pctiles": {
            "p50": int(np.percentile(tuws, 50)),
            "p95": int(np.percentile(tuws, 95)),
            "p99": int(np.percentile(tuws, 99)),
        },
        "tuw_p95": int(np.percentile(tuws, 95)),
        "ruin_prob_by_capital": ruin,
        "ruin_line": ", ".join(f"{k} drawdown tolerance -> {v}" for k, v in ruin.items()),
        "usable_as_capital_requirement": mode != "reshuffle",
        "provenance": provenance(),
        "seed": seed,
    }

    if mode == "reshuffle":
        # Deliberately absent, and named as absent. An empty field invites the
        # reader to fill it in; a stated refusal does not.
        detail["endpoint_dispersion"] = None
        detail["endpoint_line"] = (
            "Endpoint dispersion is not reported in reshuffle mode: every path ends at "
            f"{round(100.0 * float(endpoints[0]), 2)}% by construction."
        )
    else:
        detail["endpoint_dispersion"] = {
            "p5": pct(endpoints, 5),
            "p50": pct(endpoints, 50),
            "p95": pct(endpoints, 95),
            "prob_negative": round(float(np.mean(endpoints < 0)), 4),
        }
        detail["endpoint_line"] = (
            f"Endpoint p5 {pct(endpoints, 5)}%, p50 {pct(endpoints, 50)}%, "
            f"p95 {pct(endpoints, 95)}%; probability of a losing run "
            f"{round(float(np.mean(endpoints < 0)), 4)}."
        )

    print(json.dumps({"summary": None, "n": n, "detail": detail}))


if __name__ == "__main__":
    main()
