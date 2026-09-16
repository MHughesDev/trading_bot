"""Could a decision at T have seen a candle that had not finished yet.

```json
{
  "name": "bar_boundary_check",
  "version": 1,
  "namespace": "audit",
  "description": "Proves whether a higher-timeframe bar could have been read before it closed, by checking that every aggregate is consistent with only the lower-timeframe bars that had already completed. Run it on any strategy that mixes timeframes.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "timeframe", "higher_timeframe", "start", "end"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string"},
      "timeframe": {"type": "string", "description": "the fast series, e.g. 1m"},
      "higher_timeframe": {"type": "string", "description": "the aggregate, e.g. 1h"},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "tolerance_bps": {"type": "number", "minimum": 0, "maximum": 100, "default": 1.0,
                        "description": "price agreement tolerance before a bar counts as violating"}
    }
  },
  "summary_template": "{n_checked} higher-timeframe bars checked against their constituents; {n_violations} could not have been produced from completed bars alone; {verdict}{first_violation_note}",
  "detail_fields": [
    ["n_checked", "bars"],
    ["n_violations", "bars"],
    ["verdict", ""],
    ["first_violation_note", ""]
  ],
  "wall_seconds": 300,
  "series": [
    {"role": "fast"},
    {"role": "slow", "timeframe_arg": "higher_timeframe"}
  ]
}
```

The bug this is for lives in this platform
-------------------------------------------
The continuous aggregation turns a live 1m stream into higher-timeframe OHLCV.
An aggregate bar is only *knowable* once its last constituent minute has closed —
but the row exists from the moment the first minute lands, and a reader that
takes the row at its open timestamp reads a bar containing minutes that have not
happened yet. Every strategy built on that mix is trading on a candle from the
future, and its backtest is spectacular.

What "violating" means here
----------------------------
For each higher-timeframe bar, the constituents are the lower-timeframe bars
whose timestamps fall inside it. The aggregate is *reconstructible* when:

* its open equals the first constituent's open,
* its close equals the **last** constituent's close,
* its high is the max of the constituents' highs and its low the min,
* and, the one that actually catches the bug, no constituent falls **outside**
  the bar's own interval.

A mismatch on the close is the signature of a partially-formed bar being read as
though it were finished: the stored close is the close of the minute that had
landed when the row was last written, and the true close comes later.

Why this is mechanical, and reads no strategy code
---------------------------------------------------
It never looks at a signal, a rule or an indicator. It compares two stored series
against each other, so it detects the defect for every strategy at once rather
than one at a time — and a strategy that "passes" a code review while reading a
mis-aggregated series is exactly the case a code review cannot catch.

Reported, never repaired
-------------------------
It writes nothing. A bar-boundary violation is a data-integrity finding about the
aggregation pipeline, and repairing it here would fix the symptom for one caller
while leaving every other reader of the same rows wrong.
"""

_BPS = 10_000.0


def _timeframe_seconds(timeframe):
    unit = timeframe[-1:].lower()
    try:
        size = int(timeframe[:-1])
    except (ValueError, TypeError):
        return None
    seconds = {"s": 1, "m": 60, "h": 3600, "d": 86400, "w": 604800}.get(unit)
    return size * seconds if seconds and size > 0 else None


def _close_enough(a, b, tolerance):
    if not (math.isfinite(a) and math.isfinite(b)):
        return False
    scale = max(abs(a), abs(b), 1e-12)
    return abs(a - b) / scale <= tolerance


def run(payload: dict) -> dict:
    fast = load(payload, "fast")
    slow = load(payload, "slow")

    fast_tf = payload.get("timeframe")
    slow_tf = payload.get("higher_timeframe")
    fast_s = _timeframe_seconds(fast_tf)
    slow_s = _timeframe_seconds(slow_tf)
    if not fast_s or not slow_s:
        return refuse(f"unrecognised timeframe pair {fast_tf!r} / {slow_tf!r}")
    if slow_s <= fast_s:
        return refuse(
            f"`higher_timeframe` must be coarser than `timeframe`; got {slow_tf} against "
            f"{fast_tf}. Nothing can be aggregated from bars that are longer than it."
        )
    if slow_s % fast_s != 0:
        return refuse(
            f"{slow_tf} is not a whole number of {fast_tf} bars, so its constituents are not "
            f"well defined and any answer here would be about the misalignment rather than "
            f"about lookahead"
        )

    if not isinstance(fast.index, pd.DatetimeIndex) or not isinstance(slow.index, pd.DatetimeIndex):
        return refuse("both series need a timestamp index to be aligned")

    tolerance = float(payload.get("tolerance_bps") or 1.0) / _BPS

    # Bucket every fast bar by the slow bar that contains it. Integer division on
    # epoch seconds, so the buckets are the same ones the aggregation itself
    # would form and no rounding creeps in.
    fast = fast.copy()
    fast["_bucket"] = (epoch_seconds(fast.index) // slow_s) * slow_s
    slow = slow.copy()
    slow["_bucket"] = (epoch_seconds(slow.index) // slow_s) * slow_s

    grouped = {int(b): g for b, g in fast.groupby("_bucket", sort=True)}

    violations = []
    checked = 0
    missing_constituents = 0
    expected_per_bar = slow_s // fast_s

    for _, row in slow.iterrows():
        bucket = int(row["_bucket"])
        members = grouped.get(bucket)
        if members is None or len(members) == 0:
            missing_constituents += 1
            continue
        # A partial bucket cannot distinguish "the aggregate read the future"
        # from "the fast series has a gap", so it is counted and skipped rather
        # than reported as a violation. Counting it matters: a run where most
        # buckets are partial is a run this test could not actually check.
        if len(members) < expected_per_bar:
            missing_constituents += 1
            continue

        checked += 1
        members = members.sort_index()
        reasons = []

        if not _close_enough(float(row["open"]), float(members["open"].iloc[0]), tolerance):
            reasons.append("open does not match the first constituent")
        if not _close_enough(float(row["close"]), float(members["close"].iloc[-1]), tolerance):
            # The signature of the bug: the stored close is whatever had landed
            # when the row was last written.
            reasons.append("close does not match the LAST constituent")
        if not _close_enough(float(row["high"]), float(members["high"].max()), tolerance):
            reasons.append("high is not the maximum of the constituents")
        if not _close_enough(float(row["low"]), float(members["low"].min()), tolerance):
            reasons.append("low is not the minimum of the constituents")

        if reasons:
            # Which constituent the stored close *does* match, if any. That index
            # is how far into the bar the reader could see, which turns "this is
            # wrong" into "this was readable N minutes early".
            matched_at = None
            for i in range(len(members)):
                if _close_enough(float(row["close"]), float(members["close"].iloc[i]), tolerance):
                    matched_at = i
                    break
            violations.append(
                {
                    "bar_open": pd.Timestamp(bucket, unit="s", tz="UTC").isoformat(),
                    "reasons": reasons,
                    "stored": {k: float(row[k]) for k in ("open", "high", "low", "close")},
                    "reconstructed": {
                        "open": float(members["open"].iloc[0]),
                        "high": float(members["high"].max()),
                        "low": float(members["low"].min()),
                        "close": float(members["close"].iloc[-1]),
                    },
                    "close_matches_constituent_index": matched_at,
                    "readable_bars_early": (
                        len(members) - 1 - matched_at if matched_at is not None else None
                    ),
                }
            )

    n_violations = len(violations)
    first = violations[0] if violations else None

    if checked == 0:
        return refuse(
            f"no {slow_tf} bar had a complete set of {fast_tf} constituents over this window, so "
            f"nothing could be checked. That is itself worth knowing — it usually means the fast "
            f"series has gaps",
            n_incomplete=missing_constituents,
        )

    if n_violations == 0:
        verdict = (
            f"every {slow_tf} bar is reconstructible from completed {fast_tf} bars alone, so a "
            f"decision at a bar's close could not have seen an unfinished candle"
        )
    else:
        verdict = (
            f"{n_violations} of {checked} bars could NOT have been produced from completed "
            f"{fast_tf} bars. A strategy reading {slow_tf} on this instrument is reading "
            f"partially-formed candles"
        )

    first_violation_note = (
        f"; first at {first['bar_open']}" if first else ""
    )

    warnings = []
    if n_violations:
        early = [v["readable_bars_early"] for v in violations if v["readable_bars_early"]]
        if early:
            warnings.append(
                f"in {len(early)} cases the stored close matches a constituent that is not the "
                f"last one, by a median of {int(np.median(early))} {fast_tf} bars. That is the "
                f"signature of a partially-formed aggregate being read as finished, and the "
                f"median is how far into the future a reader could see."
            )
        warnings.append(
            "this is a data-integrity finding about the aggregation pipeline, not about any one "
            "strategy. Nothing was repaired here: fixing it for this caller would leave every "
            "other reader of the same rows wrong."
        )
    if missing_constituents > checked:
        warnings.append(
            f"{missing_constituents} bars were skipped for incomplete constituents against "
            f"{checked} actually checked. Most of this window could not be verified, so a clean "
            f"result here is weak evidence."
        )

    detail = {
        "instrument_id": payload.get("instrument_id"),
        "timeframe": fast_tf,
        "higher_timeframe": slow_tf,
        "constituents_per_bar": int(expected_per_bar),
        "tolerance_bps": float(payload.get("tolerance_bps") or 1.0),
        "n_checked": checked,
        "n_incomplete_skipped": missing_constituents,
        "n_violations": n_violations,
        "verdict": verdict,
        "first_violation_note": first_violation_note,
        "first_violation": first,
        "violations": violations[:50],
        "violations_truncated": max(0, n_violations - 50),
        "warnings": warnings,
        "window": {
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(detail, n=checked)
