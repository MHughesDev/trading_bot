"""
Derived OHLC intervals from a **canonical** fine-grained series (FB-AP-017).

**Storage:** QuestDB retains **native** ``interval_seconds`` rows (e.g. 1s from live/REST).
**Charts** and **training** derive coarser bars **on the fly** with the same Polars OHLC rules
(left-aligned UTC buckets: open=first, high=max, low=min, close=last, volume=sum).

Do **not** duplicate rollup logic elsewhere — import from this module.
"""

from __future__ import annotations

import polars as pl

# Canonical seconds resolution for live + REST (see ADR / FB-AP-014); charts assume 1s rows exist.
CANONICAL_BASE_INTERVAL_SECONDS: int = 1

# UI / chart presets (aligned with ``control_plane.asset_chart_data.preset_to_request``).
CHART_PRESET_INTERVAL_SECONDS: dict[str, int] = {
    "second": 1,
    "minute": 60,
    "hour": 3600,
    "day": 86_400,
}


def resample_ohlc(
    df: pl.DataFrame,
    *,
    bucket_seconds: int,
    ts_column: str = "ts",
) -> pl.DataFrame:
    """OHLCV resample (left-aligned UTC buckets)."""
    if df.height == 0:
        return df
    every = f"{max(1, int(bucket_seconds))}s"
    return (
        df.sort(ts_column)
        .group_by_dynamic(ts_column, every=every, closed="left")
        .agg(
            pl.col("open").first().alias("open"),
            pl.col("high").max().alias("high"),
            pl.col("low").min().alias("low"),
            pl.col("close").last().alias("close"),
            pl.col("volume").sum().alias("volume"),
        )
        .sort(ts_column)
    )


def resample_weekly_from_daily(df_daily: pl.DataFrame, *, ts_column: str = "ts") -> pl.DataFrame:
    """Aggregate daily bars to calendar weeks (``group_by_dynamic`` every ``7d``)."""
    if df_daily.height == 0:
        return df_daily
    return (
        df_daily.sort(ts_column)
        .group_by_dynamic(ts_column, every="7d", closed="left")
        .agg(
            pl.col("open").first().alias("open"),
            pl.col("high").max().alias("high"),
            pl.col("low").min().alias("low"),
            pl.col("close").last().alias("close"),
            pl.col("volume").sum().alias("volume"),
        )
        .sort(ts_column)
    )


def daily_ohlc_aligned_with_training(
    df_fine: pl.DataFrame,
    *,
    training_granularity_seconds: int,
    ts_column: str = "ts",
) -> pl.DataFrame:
    """
    From fine OHLC (e.g. 1s canonical rows), build **daily** bars.

    When ``training_granularity_seconds`` > 1 and < 86400, aggregate to that step **before**
    daily rollup so charts match :func:`orchestration.real_data_bars.fetch_symbol_bars_sync` /
    training campaign bar geometry (**FB-AP-017**).
    """
    if df_fine.height == 0:
        return df_fine
    g = max(1, int(training_granularity_seconds))
    work = df_fine
    if ts_column != "ts":
        work = work.rename({ts_column: "ts"})
    if g > 1 and g < 86_400:
        work = resample_ohlc(work, bucket_seconds=g, ts_column="ts")
    return resample_ohlc(work, bucket_seconds=86_400, ts_column="ts")


def resample_monthly_from_daily(df_daily: pl.DataFrame, *, ts_column: str = "ts") -> pl.DataFrame:
    """Aggregate daily bars to calendar months (``every='1mo'``)."""
    if df_daily.height == 0:
        return df_daily
    return (
        df_daily.sort(ts_column)
        .group_by_dynamic(ts_column, every="1mo", closed="left")
        .agg(
            pl.col("open").first().alias("open"),
            pl.col("high").max().alias("high"),
            pl.col("low").min().alias("low"),
            pl.col("close").last().alias("close"),
            pl.col("volume").sum().alias("volume"),
        )
        .sort(ts_column)
    )
