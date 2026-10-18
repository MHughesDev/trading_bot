"""Where a strategy's return came from: drift, volatility timing, or selection.

```json
{
  "name": "decompose_strategy_returns",
  "version": 1,
  "namespace": "attribution",
  "description": "Splits a strategy's bar-by-bar return into drift (average exposure times the asset's average return), volatility timing (exposure explained by trailing volatility) and the residual selection component. Run it before crediting a result to skill.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "timeframe", "start", "end"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "exposure": {
        "type": "array",
        "items": {"type": "number"},
        "minItems": 30,
        "maxItems": 100000,
        "description": "the exposure held after each bar's close, one per bar of the window (a fraction of equity, signed). Omitted, a reference rule is decomposed instead and the result says so."
      },
      "reference_rule": {
        "type": "string",
        "enum": ["long", "vol_target"],
        "default": "vol_target",
        "description": "used only when exposure is omitted: constant long, or a long scaled by the inverse of trailing volatility"
      },
      "vol_window": {"type": "integer", "minimum": 5, "maximum": 1000, "default": 24, "description": "bars in the trailing volatility estimate"}
    }
  },
  "summary_template": "{n_bars} bars, total {total_pct}% = drift {drift_pct}% + volatility timing {timing_pct}% + selection {selection_pct}%; exposure from {exposure_source}, {exposure_vol_r2} of its variation explained by trailing volatility",
  "detail_fields": [
    ["n_bars", ""],
    ["total_pct", "pct"],
    ["drift_pct", "pct"],
    ["timing_pct", "pct"],
    ["selection_pct", "pct"],
    ["exposure_source", ""],
    ["exposure_vol_r2", ""],
    ["mean_exposure", ""],
    ["reading", ""]
  ],
  "wall_seconds": 60,
  "series": [{"role": "primary"}]
}
```

What this answers
-----------------
A strategy that is long all the time in a rising market has earned the market's drift and
nothing else. One that sizes down when volatility is high has earned a timing effect, which
is real and is not stock selection or signal skill. Reporting the total hides which of these a
result is. This tool splits it, exactly, into three parts that sum to the total.

The arithmetic
--------------
Let `e[k]` be the exposure held after bar k's close and `R[k]` the return of the next bar. The
strategy's return before costs is `sum(e[k] * R[k])`, a **sum of bar returns** (not a compounded
curve), and with `ebar` and `mu` the means of `e` and `R` it splits as

    sum(e R)  =  N * ebar * mu            drift: average exposure times the asset's average return
              +  sum((ehat - ebar)(R - mu))   volatility timing
              +  sum((e - ehat)(R - mu))      selection

where `ehat` is the part of exposure a linear function of trailing volatility (and of its inverse)
explains, fitted by least squares **with an intercept**, so the three parts sum to the total with no
remainder beyond rounding. Trailing volatility at bar k uses only returns of bars up to k, so the
regressor is something the strategy could have known.

Reading it
----------
* A constant exposure has no variation, so `timing` and `selection` are zero: all drift.
* An exposure that is a function of volatility is `timing`, which is non-zero when volatility and
  the asset's return are related (high volatility with falling prices, for example).
* `selection` is what is left: exposure variation volatility does not explain, which is where a
  directional signal's skill, or luck, shows up. It is a residual, not proof of skill.

What it does not do
-------------------
It is before costs, so a strategy that trades often can show a positive total and lose money. It
does not test significance and `selection` is one path's realised covariance, not an expectation.
The volatility model is linear in volatility and its inverse; an exposure rule shaped differently
leaves some timing inside `selection`, and `exposure_vol_r2` tells you how much of the exposure the
model captured.
"""

REFERENCE_NOTE = {
    "supplied": "the exposure array in the call",
    "reference:long": "a reference rule (constant long), because no exposure was supplied",
    "reference:vol_target": "a reference rule (long scaled by inverse trailing volatility), because no exposure was supplied",
}


def run(payload: dict) -> dict:
    frame = load(payload, "primary")
    if "close" not in frame.columns:
        return refuse("the bars lack `close`")
    close = frame["close"].astype(float).to_numpy()
    if not np.all(np.isfinite(close) & (close > 0)):
        return refuse("the window has a non-positive or missing close; decomposing it would be arithmetic on a hole")
    n = int(close.size)
    window = int(payload.get("vol_window") or 24)
    if n < window + 30:
        return refuse(f"{n} bars is too few for a {window}-bar volatility window and a decomposition")

    exposure = payload.get("exposure")
    r = close[1:] / close[:-1] - 1.0  # r[k] is the return of bar k+1, earned by the exposure held after bar k
    if exposure is None:
        rule = str(payload.get("reference_rule") or "vol_target")
        if rule not in ("long", "vol_target"):
            return refuse(f"`reference_rule` is long or vol_target, not {rule!r}")
        source = f"reference:{rule}"
        e_full = np.ones(n)
        if rule == "vol_target":
            vols = np.full(n, np.nan)
            for k in range(window, n - 1):
                vols[k] = float(np.std(r[k - window : k], ddof=1))
            target = float(np.nanmedian(vols))
            with np.errstate(divide="ignore", invalid="ignore"):
                e_full = np.where(vols > 1e-12, np.clip(target / vols, 0.0, 3.0), 0.0)
    else:
        source = "supplied"
        e_full = np.asarray(exposure, dtype=float)
        if e_full.size != n:
            return refuse(
                f"`exposure` has {e_full.size} values and the window has {n} bars; it is one value "
                f"per bar, the exposure held after that bar's close"
            )
        if not np.all(np.isfinite(e_full)):
            return refuse("`exposure` has a missing or non-finite value")

    ks = []
    vol = []
    for k in range(window, n - 1):
        v = float(np.std(r[k - window : k], ddof=1))
        if v > 1e-12:
            ks.append(k)
            vol.append(v)
    ks = np.asarray(ks)
    vol = np.asarray(vol)
    if ks.size < 30:
        return refuse(f"only {ks.size} bars have a usable trailing volatility; the window is too flat or too short")

    e = e_full[ks]
    ret = r[ks]
    cnt = int(ks.size)
    ebar = float(np.mean(e))
    mu = float(np.mean(ret))
    total = float(np.sum(e * ret))
    drift = cnt * ebar * mu

    design = np.column_stack([np.ones(cnt), vol, 1.0 / vol])
    coef, *_ = np.linalg.lstsq(design, e, rcond=None)
    ehat = design @ coef
    timing = float(np.sum((ehat - ebar) * (ret - mu)))
    selection = float(np.sum((e - ehat) * (ret - mu)))
    ssv = float(np.sum((e - ebar) ** 2))
    r2 = 0.0 if ssv < 1e-18 else max(0.0, min(1.0, 1.0 - float(np.sum((e - ehat) ** 2)) / ssv))

    if ssv < 1e-12:
        reading = "constant exposure: the whole return is drift, and there is no timing or selection to attribute"
    elif r2 >= 0.8:
        reading = "exposure is mostly a function of trailing volatility: read the timing term as the strategy's behaviour"
    elif r2 >= 0.2:
        reading = "exposure partly follows volatility; the selection term mixes signal skill with timing the model did not capture"
    else:
        reading = "exposure does not follow volatility; the selection term is the directional component"

    detail = {
        "instrument_id": payload.get("instrument_id"),
        "n_bars": cnt,
        "vol_window": window,
        "exposure_source": source,
        "exposure_source_note": REFERENCE_NOTE[source],
        "total_pct": total * 100.0,
        "drift_pct": drift * 100.0,
        "timing_pct": timing * 100.0,
        "selection_pct": selection * 100.0,
        "components_minus_total": (drift + timing + selection) - total,
        "mean_exposure": ebar,
        "asset_mean_return_per_bar": mu,
        "exposure_vol_r2": r2,
        "reading": reading,
        "warnings": [
            "a sum of bar returns before costs, not a compounded curve; a frequent trader can show a positive total and lose money",
            "selection is a residual covariance on one path, not evidence of skill",
        ],
    }
    return result(detail, n=cnt)
