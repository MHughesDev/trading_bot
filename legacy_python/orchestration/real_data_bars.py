"""Load real OHLCV from Kraken public REST (market data policy: Kraken-only for training)."""

from __future__ import annotations

import asyncio
import hashlib
import json
import logging
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import polars as pl

from data_plane.bars.derived_intervals import resample_ohlc as _derived_resample_ohlc_ts
from data_plane.ingest.kraken_rest import (
    KrakenOHLCRow,
    KrakenRESTClient,
    fetch_ohlc_range,
    fetch_trades_range,
    granularity_to_kraken_ohlc_interval_minutes,
)
from data_plane.ingest.kraken_symbols import kraken_rest_pair

logger = logging.getLogger(__name__)


def dataset_snapshot_id(symbol: str, start: datetime, end: datetime, granularity_seconds: int) -> str:
    """Stable id for manifests and reports (spec: data snapshot identifier)."""
    raw = f"{symbol}|{start.isoformat()}|{end.isoformat()}|{granularity_seconds}"
    return hashlib.sha256(raw.encode()).hexdigest()[:16]


def _ohlc_rows_to_polars(rows: list[KrakenOHLCRow]) -> pl.DataFrame:
    if not rows:
        return pl.DataFrame(
            schema={
                "timestamp": pl.Datetime(time_unit="us", time_zone="UTC"),
                "open": pl.Float64,
                "high": pl.Float64,
                "low": pl.Float64,
                "close": pl.Float64,
                "volume": pl.Float64,
            }
        )
    out = []
    for c in rows:
        out.append(
            {
                "timestamp": datetime.fromtimestamp(c.time, tz=UTC),
                "open": c.open,
                "high": c.high,
                "low": c.low,
                "close": c.close,
                "volume": c.volume,
            }
        )
    return pl.DataFrame(out).sort("timestamp")


def _resample_ohlc(df: pl.DataFrame, granularity_seconds: int) -> pl.DataFrame:
    """Resample OHLC to target bar size — shared rules with charts (FB-AP-017)."""
    if df.height == 0:
        return df
    out = _derived_resample_ohlc_ts(
        df.rename({"timestamp": "ts"}),
        bucket_seconds=granularity_seconds,
        ts_column="ts",
    )
    return out.rename({"ts": "timestamp"})


def _trades_to_ohlc(
    trades: list[tuple[float, float, float]],
    granularity_seconds: int,
) -> pl.DataFrame:
    """Aggregate (price, vol, time_sec_float) trades into OHLCV bars."""
    if not trades:
        return pl.DataFrame(
            schema={
                "timestamp": pl.Datetime(time_unit="us", time_zone="UTC"),
                "open": pl.Float64,
                "high": pl.Float64,
                "low": pl.Float64,
                "close": pl.Float64,
                "volume": pl.Float64,
            }
        )
    rows = [{"t": datetime.fromtimestamp(t, tz=UTC), "price": p, "size": v} for p, v, t in trades]
    df = pl.DataFrame(rows).sort("t")
    every = f"{granularity_seconds}s"
    return (
        df.group_by_dynamic("t", every=every, closed="left")
        .agg(
            pl.col("price").first().alias("open"),
            pl.col("price").max().alias("high"),
            pl.col("price").min().alias("low"),
            pl.col("price").last().alias("close"),
            pl.col("size").sum().alias("volume"),
        )
        .rename({"t": "timestamp"})
        .sort("timestamp")
    )


async def fetch_symbol_bars_async(
    symbol: str,
    start: datetime,
    end: datetime,
    *,
    granularity_seconds: int = 60,
) -> pl.DataFrame:
    """
    Fetch OHLCV for ``symbol`` (e.g. ``BTC-USD``) from Kraken.

    - If ``granularity_seconds`` matches Kraken OHLC (1m,5m,…), uses ``/public/OHLC`` with pagination.
    - Else if ``granularity_seconds`` is a multiple of 60: fetches 1m OHLC and resamples.
    - Else (sub-minute or odd sizes): uses ``/public/Trades`` and aggregates (can be slow for long ranges).
    """
    pair = kraken_rest_pair(symbol)
    client = KrakenRESTClient()
    try:
        interval_min = granularity_to_kraken_ohlc_interval_minutes(granularity_seconds)
        if interval_min is not None:
            rows = await fetch_ohlc_range(client, pair, start, end, interval_minutes=interval_min)
            df = _ohlc_rows_to_polars(rows)
        elif granularity_seconds >= 60 and granularity_seconds % 60 == 0:
            rows = await fetch_ohlc_range(client, pair, start, end, interval_minutes=1)
            df = _ohlc_rows_to_polars(rows)
            df = _resample_ohlc(df, granularity_seconds)
        else:
            logger.info(
                "fetch_symbol_bars: using Kraken /Trades for %s granularity=%ss (may be slow)",
                pair,
                granularity_seconds,
            )
            trades = await fetch_trades_range(client, pair, start, end)
            df = _trades_to_ohlc(trades, granularity_seconds)
    finally:
        await client.aclose()

    if df.height == 0:
        raise RuntimeError(f"no bars returned for {symbol} between {start} and {end}")
    return df


def fetch_symbol_bars_sync(
    symbol: str,
    start: datetime,
    end: datetime,
    *,
    granularity_seconds: int = 60,
) -> pl.DataFrame:
    return asyncio.run(
        fetch_symbol_bars_async(symbol, start, end, granularity_seconds=granularity_seconds)
    )


def write_snapshot_manifest(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, default=str), encoding="utf-8")
