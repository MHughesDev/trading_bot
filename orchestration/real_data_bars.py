"""Load real OHLCV from Coinbase public REST (same source as live market data policy)."""

from __future__ import annotations

import asyncio
import hashlib
import json
import logging
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

import polars as pl

from data_plane.ingest.coinbase_rest import CoinbaseCandle, CoinbaseRESTClient

logger = logging.getLogger(__name__)


def dataset_snapshot_id(symbol: str, start: datetime, end: datetime, granularity_seconds: int) -> str:
    """Stable id for manifests and reports (spec: data snapshot identifier)."""
    raw = f"{symbol}|{start.isoformat()}|{end.isoformat()}|{granularity_seconds}"
    return hashlib.sha256(raw.encode()).hexdigest()[:16]


async def fetch_candles_range(
    client: CoinbaseRESTClient,
    product_id: str,
    start: datetime,
    end: datetime,
    *,
    granularity_seconds: int = 60,
) -> list[CoinbaseCandle]:
    """Paginate public candles (max ~300 per request on Exchange API)."""
    start = start.replace(tzinfo=UTC) if start.tzinfo is None else start.astimezone(UTC)
    end = end.replace(tzinfo=UTC) if end.tzinfo is None else end.astimezone(UTC)
    # Public API returns at most ~300 candles per request; stay under that window.
    max_candles_per_request = 280
    chunk = timedelta(seconds=granularity_seconds * max_candles_per_request)
    all_rows: list[CoinbaseCandle] = []
    cur = start
    while cur < end:
        chunk_end = min(cur + chunk, end)
        batch = await client.get_public_candles(
            product_id, cur, chunk_end, granularity_seconds=granularity_seconds
        )
        all_rows.extend(batch)
        cur = chunk_end
    # Dedupe by time and sort
    by_t: dict[float, CoinbaseCandle] = {}
    for c in all_rows:
        by_t[c.start.timestamp()] = c
    ordered = sorted(by_t.values(), key=lambda x: x.start)
    return ordered


def candles_to_polars(candles: list[CoinbaseCandle]) -> pl.DataFrame:
    if not candles:
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
    rows = [
        {
            "timestamp": c.start,
            "open": c.open,
            "high": c.high,
            "low": c.low,
            "close": c.close,
            "volume": c.volume,
        }
        for c in candles
    ]
    return pl.DataFrame(rows).sort("timestamp")


async def fetch_symbol_bars_async(
    symbol: str,
    start: datetime,
    end: datetime,
    *,
    granularity_seconds: int = 60,
) -> pl.DataFrame:
    client = CoinbaseRESTClient()
    try:
        candles = await fetch_candles_range(
            client, symbol, start, end, granularity_seconds=granularity_seconds
        )
    finally:
        await client.aclose()
    df = candles_to_polars(candles)
    if df.height == 0:
        raise RuntimeError(f"no candles returned for {symbol} between {start} and {end}")
    return df


def fetch_symbol_bars_sync(
    symbol: str,
    start: datetime,
    end: datetime,
    *,
    granularity_seconds: int = 60,
) -> pl.DataFrame:
    return asyncio.run(fetch_symbol_bars_async(symbol, start, end, granularity_seconds=granularity_seconds))


def write_snapshot_manifest(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, default=str), encoding="utf-8")
