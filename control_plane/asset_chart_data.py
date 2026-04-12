"""Chart bar loading and interval resampling for Asset Page (FB-AP-028).

Fetches ``GET /assets/chart/bars`` at a **native** ``interval_seconds`` when possible; if the
response is empty, falls back to finer bars and **resamples** with Polars (week/month from daily
OHLC via ``group_by_dynamic``).
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from typing import Any, Literal

import polars as pl

from data_plane.bars.derived_intervals import (
    resample_monthly_from_daily as _resample_monthly_from_daily,
    resample_ohlc as _resample_ohlc_core,
    resample_weekly_from_daily as _resample_weekly_from_daily,
)

Preset = Literal["second", "minute", "hour", "day", "week", "month"]


@dataclass(frozen=True)
class ChartRequest:
    interval_seconds: int
    window: timedelta
    limit: int


def preset_to_request(preset: Preset) -> ChartRequest:
    """Target QuestDB row shape; coarse presets use fewer rows per window."""
    if preset == "second":
        return ChartRequest(interval_seconds=1, window=timedelta(hours=2), limit=8_000)
    if preset == "minute":
        return ChartRequest(interval_seconds=60, window=timedelta(days=7), limit=20_000)
    if preset == "hour":
        return ChartRequest(interval_seconds=3600, window=timedelta(days=45), limit=12_000)
    if preset == "day":
        return ChartRequest(interval_seconds=86_400, window=timedelta(days=730), limit=800)
    if preset == "week":
        return ChartRequest(interval_seconds=86_400, window=timedelta(days=730), limit=800)
    if preset == "month":
        return ChartRequest(interval_seconds=86_400, window=timedelta(days=1825), limit=800)
    raise ValueError(f"unknown preset {preset!r}")


def _bars_payload_to_df(bars: list[dict[str, Any]]) -> pl.DataFrame:
    if not bars:
        return pl.DataFrame(
            schema={
                "ts": pl.Datetime(time_zone="UTC"),
                "open": pl.Float64,
                "high": pl.Float64,
                "low": pl.Float64,
                "close": pl.Float64,
                "volume": pl.Float64,
            }
        )
    rows = []
    for b in bars:
        raw = b.get("ts")
        if isinstance(raw, str):
            t = datetime.fromisoformat(raw.replace("Z", "+00:00"))
        else:
            continue
        rows.append(
            {
                "ts": t,
                "open": float(b["open"]),
                "high": float(b["high"]),
                "low": float(b["low"]),
                "close": float(b["close"]),
                "volume": float(b.get("volume") or 0.0),
            }
        )
    return pl.DataFrame(rows).sort("ts")


def resample_ohlc(df: pl.DataFrame, *, bucket_seconds: int) -> pl.DataFrame:
    """OHLCV resample — delegates to :mod:`data_plane.bars.derived_intervals` (FB-AP-017)."""
    return _resample_ohlc_core(df, bucket_seconds=bucket_seconds, ts_column="ts")


def resample_weekly_ohlc(df_daily: pl.DataFrame) -> pl.DataFrame:
    """Weekly from daily — shared rollup (FB-AP-017)."""
    return _resample_weekly_from_daily(df_daily, ts_column="ts")


def resample_monthly_ohlc(df_daily: pl.DataFrame) -> pl.DataFrame:
    """Monthly from daily — shared rollup (FB-AP-017)."""
    return _resample_monthly_from_daily(df_daily, ts_column="ts")


def bars_dicts_from_df(df: pl.DataFrame) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for row in df.iter_rows(named=True):
        ts = row["ts"]
        if hasattr(ts, "isoformat"):
            tss = ts.isoformat() if ts.tzinfo else ts.replace(tzinfo=UTC).isoformat()
        else:
            tss = str(ts)
        out.append(
            {
                "ts": tss,
                "open": float(row["open"]),
                "high": float(row["high"]),
                "low": float(row["low"]),
                "close": float(row["close"]),
                "volume": float(row.get("volume") or 0.0),
            }
        )
    return out
