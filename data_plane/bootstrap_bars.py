"""
Clean and validate Kraken bootstrap OHLCV for per-asset init (FB-AP-008).

**Canonical bar shape** matches :mod:`orchestration.real_data_bars` / storage expectations:
``timestamp`` (UTC), ``open``, ``high``, ``low``, ``close``, ``volume`` — see
:class:`data_plane.storage.questdb.QuestDBWriter` / :class:`app.contracts.events.BarEvent`.

**Outlier policy (init bootstrap only)**

- Rows with **invalid geometry** (``high < low``), **negative volume**, or **OHLC inconsistent
  with bounds** (open/close outside ``[low, high]`` beyond a tiny float tolerance) are **dropped**
  and counted; the job step detail records ``drop_invalid_*`` counts.
- Extreme **prices or volumes** are **not** winsorized or removed — only structural invalidity.
- **Gap detection** does not fail the step: missing intervals are **counted** and logged in
  detail (``gap_intervals``, ``max_gap_seconds``) for operator review; Kraken/API gaps are common.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

import polars as pl

CANONICAL_BAR_COLUMNS: tuple[str, ...] = (
    "timestamp",
    "open",
    "high",
    "low",
    "close",
    "volume",
)


@dataclass(frozen=True)
class InitBootstrapValidationResult:
    """Result of validate/clean; ``cleaned`` is sorted by ``timestamp`` ascending."""

    cleaned: pl.DataFrame
    input_rows: int
    output_rows: int
    duplicates_removed: int
    gap_intervals: int
    max_gap_seconds: float | None
    dropped_high_lt_low: int
    dropped_negative_volume: int
    dropped_ohlc_inconsistent: int


def _ensure_utc_timestamp(df: pl.DataFrame) -> pl.DataFrame:
    """Normalize ``timestamp`` to timezone-aware UTC (Polars datetime)."""
    ts = pl.col("timestamp")
    tz = df.schema.get("timestamp")
    if isinstance(tz, pl.Datetime) and tz.time_zone is None:
        return df.with_columns(ts.dt.replace_time_zone("UTC"))
    return df.with_columns(ts.dt.convert_time_zone("UTC"))


def _cast_numeric(df: pl.DataFrame) -> pl.DataFrame:
    return df.with_columns(
        pl.col("open").cast(pl.Float64),
        pl.col("high").cast(pl.Float64),
        pl.col("low").cast(pl.Float64),
        pl.col("close").cast(pl.Float64),
        pl.col("volume").cast(pl.Float64),
    )


def validate_and_clean_init_bootstrap_bars(
    df: pl.DataFrame,
    *,
    granularity_seconds: int,
) -> InitBootstrapValidationResult:
    """
    Validate schema, drop duplicate timestamps (keep last), detect gaps, apply outlier policy.

    Raises:
        ValueError: empty input, missing columns, or no rows left after cleaning.
    """
    if df.height == 0:
        raise ValueError("bootstrap bars dataframe is empty")

    missing = [c for c in CANONICAL_BAR_COLUMNS if c not in df.columns]
    if missing:
        raise ValueError(f"bootstrap bars missing columns: {missing}")

    gran = max(1, int(granularity_seconds))
    input_rows = int(df.height)

    work = df.select(list(CANONICAL_BAR_COLUMNS)).pipe(_ensure_utc_timestamp).pipe(_cast_numeric)
    work = work.sort("timestamp")

    before_dedup = work.height
    work = work.unique(subset=["timestamp"], keep="last").sort("timestamp")
    duplicates_removed = before_dedup - work.height

    eps = 1e-9
    low, high = pl.col("low"), pl.col("high")
    o, c, v = pl.col("open"), pl.col("close"), pl.col("volume")

    bad_high_low = high < low
    bad_vol = v < 0
    bad_bounds = (
        (o - high > eps)
        | (low - o > eps)
        | (c - high > eps)
        | (low - c > eps)
    )

    dropped_hl = int(work.filter(bad_high_low).height)
    dropped_nv = int(work.filter(bad_vol & ~bad_high_low).height)
    dropped_ohlc = int(work.filter(bad_bounds & ~bad_high_low & ~bad_vol).height)

    work = work.filter(~bad_high_low & ~bad_vol & ~bad_bounds).sort("timestamp")

    if work.height == 0:
        raise ValueError("no rows left after removing invalid bootstrap bars")

    gap_intervals = 0
    max_gap_seconds: float | None = None
    if work.height >= 2:
        gaps = work.select(pl.col("timestamp").diff().dt.total_seconds().alias("gap_s")).drop_nulls()
        if gaps.height > 0:
            expected = float(gran)
            gap_intervals = int((gaps["gap_s"] > expected * 1.01).sum())
            max_gap_seconds = float(gaps["gap_s"].max())

    return InitBootstrapValidationResult(
        cleaned=work,
        input_rows=input_rows,
        output_rows=int(work.height),
        duplicates_removed=duplicates_removed,
        gap_intervals=gap_intervals,
        max_gap_seconds=max_gap_seconds,
        dropped_high_lt_low=dropped_hl,
        dropped_negative_volume=dropped_nv,
        dropped_ohlc_inconsistent=dropped_ohlc,
    )


def init_bootstrap_validation_detail_payload(
    result: InitBootstrapValidationResult,
    *,
    granularity_seconds: int,
) -> dict[str, Any]:
    """JSON-serializable summary for init job step detail."""
    return {
        "input_rows": result.input_rows,
        "output_rows": result.output_rows,
        "duplicates_removed": result.duplicates_removed,
        "granularity_seconds": granularity_seconds,
        "gap_intervals": result.gap_intervals,
        "max_gap_seconds": result.max_gap_seconds,
        "dropped_high_lt_low": result.dropped_high_lt_low,
        "dropped_negative_volume": result.dropped_negative_volume,
        "dropped_ohlc_inconsistent": result.dropped_ohlc_inconsistent,
    }
