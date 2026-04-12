"""Tests for FB-AP-008 bootstrap bar clean/validate."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta

import polars as pl
import pytest

from data_plane.bootstrap_bars import (
    init_bootstrap_validation_detail_payload,
    validate_and_clean_init_bootstrap_bars,
)


def _row(ts: datetime, o: float, h: float, lo: float, c: float, v: float) -> dict:
    return {
        "timestamp": ts,
        "open": o,
        "high": h,
        "low": lo,
        "close": c,
        "volume": v,
    }


def test_validate_schema_missing_column_raises() -> None:
    df = pl.DataFrame({"timestamp": [datetime(2026, 1, 1, tzinfo=UTC)], "open": [1.0]})
    with pytest.raises(ValueError, match="missing columns"):
        validate_and_clean_init_bootstrap_bars(df, granularity_seconds=60)


def test_validate_empty_raises() -> None:
    df = pl.DataFrame(
        schema={
            "timestamp": pl.Datetime(time_unit="us", time_zone="UTC"),
            "open": pl.Float64,
            "high": pl.Float64,
            "low": pl.Float64,
            "close": pl.Float64,
            "volume": pl.Float64,
        }
    )
    with pytest.raises(ValueError, match="empty"):
        validate_and_clean_init_bootstrap_bars(df, granularity_seconds=60)


def test_dedup_keeps_last() -> None:
    t = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)
    df = pl.DataFrame(
        [
            _row(t, 1.0, 1.0, 1.0, 1.0, 1.0),
            _row(t, 2.0, 2.0, 2.0, 2.0, 2.0),
        ]
    )
    r = validate_and_clean_init_bootstrap_bars(df, granularity_seconds=60)
    assert r.duplicates_removed == 1
    assert r.output_rows == 1
    assert float(r.cleaned["close"][0]) == 2.0


def test_gap_detection() -> None:
    t0 = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)
    df = pl.DataFrame(
        [
            _row(t0, 1.0, 1.0, 1.0, 1.0, 1.0),
            _row(t0 + timedelta(minutes=1), 1.0, 1.0, 1.0, 1.0, 1.0),
            _row(t0 + timedelta(minutes=4), 1.0, 1.0, 1.0, 1.0, 1.0),
        ]
    )
    r = validate_and_clean_init_bootstrap_bars(df, granularity_seconds=60)
    assert r.gap_intervals == 1
    assert r.max_gap_seconds == pytest.approx(180.0)


def test_drop_high_lt_low() -> None:
    t0 = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)
    df = pl.DataFrame(
        [
            _row(t0, 1.0, 0.5, 1.0, 1.0, 1.0),
            _row(t0 + timedelta(minutes=1), 1.0, 1.1, 1.0, 1.05, 1.0),
        ]
    )
    r = validate_and_clean_init_bootstrap_bars(df, granularity_seconds=60)
    assert r.dropped_high_lt_low == 1
    assert r.output_rows == 1


def test_drop_negative_volume() -> None:
    t0 = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)
    df = pl.DataFrame(
        [
            _row(t0, 1.0, 1.0, 1.0, 1.0, -1.0),
            _row(t0 + timedelta(minutes=1), 1.0, 1.0, 1.0, 1.0, 1.0),
        ]
    )
    r = validate_and_clean_init_bootstrap_bars(df, granularity_seconds=60)
    assert r.dropped_negative_volume == 1
    assert r.output_rows == 1


def test_naive_timestamp_assumed_utc() -> None:
    t = datetime(2026, 1, 1, 12, 0, 0)
    df = pl.DataFrame([_row(t, 1.0, 1.0, 1.0, 1.0, 1.0)])
    r = validate_and_clean_init_bootstrap_bars(df, granularity_seconds=60)
    assert r.cleaned.schema["timestamp"] == pl.Datetime(time_unit="us", time_zone="UTC")


def test_detail_payload_json_safe() -> None:
    from data_plane.bootstrap_bars import InitBootstrapValidationResult

    empty = pl.DataFrame(
        schema={
            "timestamp": pl.Datetime(time_unit="us", time_zone="UTC"),
            "open": pl.Float64,
            "high": pl.Float64,
            "low": pl.Float64,
            "close": pl.Float64,
            "volume": pl.Float64,
        }
    )
    r = InitBootstrapValidationResult(
        cleaned=empty,
        input_rows=1,
        output_rows=1,
        duplicates_removed=0,
        gap_intervals=0,
        max_gap_seconds=None,
        dropped_high_lt_low=0,
        dropped_negative_volume=0,
        dropped_ohlc_inconsistent=0,
    )
    d = init_bootstrap_validation_detail_payload(r, granularity_seconds=60)
    assert d["gap_intervals"] == 0
    assert d["max_gap_seconds"] is None
