"""How fast a model's signal stops predicting, and so how often to act on it.

```json
{
  "name": "signal_half_life",
  "version": 1,
  "namespace": "attribution",
  "description": "Rank correlation (IC) between a model version's answer and the realised log return over several horizons, a fitted exponential half-life, and the rebalance interval it implies. Reads the filled prediction series; spends no trial.",
  "input_schema": {
    "type": "object",
    "required": ["model", "instrument_id", "timeframe", "start", "end"],
    "additionalProperties": false,
    "properties": {
      "model": {"type": "string", "description": "name@version; series must be filled"},
      "output": {"type": "string", "description": "scalar output; omit if only one"},
      "instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "horizons": {"type": "array", "items": {"type": "integer", "minimum": 1, "maximum": 2000},
                   "maxItems": 12, "description": "bars; default 1,2,4,8,16,24,48,96"},
      "breakeven_rebalance_bars": {"type": "number", "minimum": 1,
                   "description": "fastest rebalance costs allow; a faster recommendation is flagged"}
    }
  },
  "summary_template": "IC {ic_h_first} at {first_horizon} bar(s), {n_significant} of {n_horizons} horizons significant; half-life {half_life_bars} bars, recommended rebalance {recommended_rebalance_bars} bars; {verdict}",
  "detail_fields": [
    ["ic_h_first", ""],
    ["first_horizon", "bars"],
    ["n_significant", "horizons"],
    ["n_horizons", "horizons"],
    ["half_life_bars", "bars"],
    ["recommended_rebalance_bars", "bars"],
    ["verdict", ""]
  ],
  "wall_seconds": 120,
  "series": [{"role": "predictions", "kind": "predictions"}]
}
```

What the label is, and what it is not
-------------------------------------
The prediction series stores what the model answered and nothing about what
happened next. The label here is derived from the bars: the log return from the
close of the last bar the decision could see to the close `h` bars later. That
makes the model's scalar a forecast of the forward return; a model whose output
means something else (a probability, a volatility) has no such label, and the
caller says which output is being treated as a return forecast.

Overlapping horizons
--------------------
At horizon `h` consecutive labels share `h - 1` of `h` bars, so `n` rows are
about `n / h` independent observations. The standard error of an IC is
`1 / sqrt(n / h - 3)` on that effective count, and a horizon is `significant`
only at `|t| >= 2`. Quoting `1/sqrt(n)` would call every long horizon
significant, because the overlap manufactures the agreement.

The half-life
-------------
`IC(h) ~ IC(0) * 2^(-h / half_life)`, fitted by weighted least squares on
`ln IC` over the horizons that are significant **and positive**, weighted by
`t^2` so the noisy far horizons that happen to be positive carry almost no say.
Fewer than three such horizons, or a non-negative slope, and the half-life is
`null` with the reason: a curve that does not decay has no half-life, and a
fitted number there would be the line's noise. The recommended rebalance is the
half-life, in bars, and is flagged when it is faster than the caller's
`breakeven_rebalance_bars`.
"""

_DEFAULT_HORIZONS = [1, 2, 4, 8, 16, 24, 48, 96]


def _spearman(a, b):
    ra = pd.Series(a).rank(method="average").to_numpy(dtype=float)
    rb = pd.Series(b).rank(method="average").to_numpy(dtype=float)
    if np.std(ra) <= 0 or np.std(rb) <= 0:
        return float("nan")
    return float(np.corrcoef(ra, rb)[0, 1])


def run(payload: dict) -> dict:
    frame = pd.read_parquet((payload.get("data") or {}).get("predictions"))
    horizons = payload.get("horizons") or _DEFAULT_HORIZONS
    horizons = sorted({int(h) for h in horizons})
    breakeven = payload.get("breakeven_rebalance_bars")

    if "y_hat" not in frame.columns or len(frame) < 100:
        return refuse(
            "fewer than 100 answered decisions in this window; a rank correlation over fewer is "
            "about which decisions happened to be answered",
            n_answers=int(len(frame)),
        )
    y_hat = pd.to_numeric(frame["y_hat"], errors="coerce").to_numpy(dtype=float)

    curve = []
    for h in horizons:
        col = f"y_h{h}"
        if col not in frame.columns:
            curve.append({"horizon": h, "ic": None, "n": 0, "reason": "no label column"})
            continue
        y = pd.to_numeric(frame[col], errors="coerce").to_numpy(dtype=float)
        ok = np.isfinite(y) & np.isfinite(y_hat)
        n = int(ok.sum())
        if n < 50:
            curve.append({"horizon": h, "ic": None, "n": n, "reason": "fewer than 50 labelled decisions"})
            continue
        ic = _spearman(y_hat[ok], y[ok])
        n_eff = n / h
        se = 1.0 / math.sqrt(n_eff - 3.0) if n_eff > 3.0 else float("nan")
        t = ic / se if np.isfinite(se) and se > 0 else float("nan")
        curve.append(
            {
                "horizon": h,
                "ic": ic,
                "n": n,
                "n_effective": n_eff,
                "se": se,
                "t": t,
                "significant": bool(np.isfinite(t) and abs(t) >= 2.0),
            }
        )

    measured = [c for c in curve if c.get("ic") is not None and np.isfinite(c["ic"])]
    if not measured:
        return refuse(
            "no horizon had enough labelled decisions; the window may end too close to the "
            "research cutoff for any forward return to be readable",
            curve=curve,
        )
    n_sig = sum(1 for c in measured if c.get("significant"))

    # Exponential fit on significant, positive horizons.
    pts = [c for c in measured if c.get("significant") and c["ic"] > 0]
    half_life, fit_note, slope = None, None, None
    if len(pts) < 3:
        fit_note = (
            f"only {len(pts)} horizon(s) have a significant positive IC; a half-life needs at "
            f"least three points to tell a decay from two coincidences"
        )
    else:
        h = np.array([c["horizon"] for c in pts], dtype=float)
        lny = np.log(np.array([c["ic"] for c in pts], dtype=float))
        w = np.array([c["t"] ** 2 for c in pts], dtype=float)
        sw = w.sum()
        hm, ym = (w * h).sum() / sw, (w * lny).sum() / sw
        var = (w * (h - hm) ** 2).sum()
        if var <= 0:
            fit_note = "the significant horizons are all the same horizon"
        else:
            slope = float((w * (h - hm) * (lny - ym)).sum() / var)
            if slope >= 0:
                fit_note = (
                    "the IC does not decay across the significant horizons (fitted slope "
                    f"{slope:+.4f} per bar), so there is no half-life to report"
                )
                slope = None
            else:
                half_life = float(math.log(2.0) / -slope)

    recommended = half_life
    faster = bool(
        recommended is not None and breakeven is not None and recommended < float(breakeven)
    )

    if n_sig == 0:
        verdict = "no_detectable_signal"
    elif half_life is None:
        verdict = "signal_present_half_life_not_estimable"
    elif faster:
        verdict = "rebalance_faster_than_costs_allow"
    else:
        verdict = "decays"

    first = measured[0]
    detail = {
        "curve": curve,
        "ic_h_first": first["ic"],
        "first_horizon": first["horizon"],
        "n_significant": n_sig,
        "n_horizons": len(horizons),
        "half_life_bars": half_life,
        "decay_model": "exponential, weighted least squares on ln IC over significant positive horizons",
        "fit_note": fit_note,
        "recommended_rebalance_bars": recommended,
        "breakeven_rebalance_bars": breakeven,
        "faster_than_breakeven": faster,
        "verdict": verdict,
        "ic_kind": "spearman",
        "n_answers": int(len(frame)),
        "window": {
            "model": payload.get("model"),
            "instrument_id": payload.get("instrument_id"),
            "timeframe": payload.get("timeframe"),
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(detail, n=int(len(frame)))
