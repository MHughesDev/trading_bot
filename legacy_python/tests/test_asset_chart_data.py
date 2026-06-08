"""FB-AP-028: asset chart presets and OHLC resampling."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta

import polars as pl

from control_plane.asset_chart_data import (
    bars_dicts_from_df,
    preset_to_request,
    resample_monthly_ohlc,
    resample_ohlc,
    resample_weekly_ohlc,
    _bars_payload_to_df,
)


def test_preset_second_is_1s() -> None:
    r = preset_to_request("second")
    assert r.interval_seconds == 1


def test_resample_ohlc_two_bars() -> None:
    t0 = datetime(2026, 4, 1, 0, 0, 0, tzinfo=UTC)
    df = pl.DataFrame(
        {
            "ts": [t0, t0 + timedelta(seconds=30)],
            "open": [1.0, 1.1],
            "high": [1.2, 1.3],
            "low": [0.9, 1.0],
            "close": [1.1, 1.15],
            "volume": [1.0, 2.0],
        }
    )
    out = resample_ohlc(df, bucket_seconds=60)
    assert out.height == 1
    assert float(out["open"][0]) == 1.0
    assert float(out["close"][0]) == 1.15


def test_monthly_from_daily() -> None:
    t0 = datetime(2026, 1, 1, 0, 0, 0, tzinfo=UTC)
    df = pl.DataFrame(
        {
            "ts": [t0, t0 + timedelta(days=5)],
            "open": [100.0, 101.0],
            "high": [105.0, 106.0],
            "low": [99.0, 100.0],
            "close": [104.0, 105.0],
            "volume": [10.0, 10.0],
        }
    )
    m = resample_monthly_ohlc(df)
    assert m.height >= 1


def test_weekly_from_daily() -> None:
    t0 = datetime(2026, 4, 6, 0, 0, 0, tzinfo=UTC)  # Monday UTC
    days = [t0 + timedelta(days=i) for i in range(3)]
    df = pl.DataFrame(
        {
            "ts": days,
            "open": [100.0, 101.0, 102.0],
            "high": [105.0, 106.0, 107.0],
            "low": [99.0, 100.0, 101.0],
            "close": [104.0, 105.0, 106.0],
            "volume": [10.0, 10.0, 10.0],
        }
    )
    w = resample_weekly_ohlc(df)
    assert w.height >= 1


def test_bars_payload_round_trip() -> None:
    bars = [
        {
            "ts": "2026-04-01T00:00:00+00:00",
            "open": 1.0,
            "high": 2.0,
            "low": 0.5,
            "close": 1.5,
            "volume": 3.0,
        }
    ]
    df = _bars_payload_to_df(bars)
    back = bars_dicts_from_df(df)
    assert len(back) == 1
    assert back[0]["close"] == 1.5
