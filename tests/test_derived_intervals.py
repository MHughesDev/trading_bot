"""FB-AP-017: shared derived OHLC rollups."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta

import polars as pl

from data_plane.bars.derived_intervals import (
    CANONICAL_BASE_INTERVAL_SECONDS,
    CHART_PRESET_INTERVAL_SECONDS,
    daily_ohlc_aligned_with_training,
    resample_ohlc,
)


def test_constants() -> None:
    assert CANONICAL_BASE_INTERVAL_SECONDS == 1
    assert CHART_PRESET_INTERVAL_SECONDS["hour"] == 3600


def test_daily_aligned_with_training_60s() -> None:
    t0 = datetime(2026, 4, 1, 0, 0, 0, tzinfo=UTC)
    # Two 1s bars in same minute
    df = pl.DataFrame(
        {
            "ts": [t0, t0 + timedelta(seconds=30)],
            "open": [100.0, 100.5],
            "high": [101.0, 101.2],
            "low": [99.0, 100.0],
            "close": [100.5, 101.0],
            "volume": [1.0, 2.0],
        }
    )
    daily = daily_ohlc_aligned_with_training(df, training_granularity_seconds=60)
    assert daily.height == 1


def test_resample_ohlc_empty() -> None:
    df = pl.DataFrame(
        schema={
            "ts": pl.Datetime(time_zone="UTC"),
            "open": pl.Float64,
            "high": pl.Float64,
            "low": pl.Float64,
            "close": pl.Float64,
            "volume": pl.Float64,
        }
    )
    assert resample_ohlc(df, bucket_seconds=60).height == 0
