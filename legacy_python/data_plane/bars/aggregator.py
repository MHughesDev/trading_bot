"""Aggregate trade ticks into OHLCV bars (Polars)."""

from __future__ import annotations

from datetime import timedelta

import polars as pl

from data_plane.ingest.normalizers import TradeTick


def trades_to_ohlcv(trades: list[TradeTick], interval: timedelta) -> pl.DataFrame:
    """Bucket trades by time window; returns columns [timestamp, open, high, low, close, volume]."""
    if not trades:
        return pl.DataFrame(
            schema={
                "timestamp": pl.Datetime(time_zone="UTC"),
                "open": pl.Float64,
                "high": pl.Float64,
                "low": pl.Float64,
                "close": pl.Float64,
                "volume": pl.Float64,
            }
        )
    rows = [
        {
            "t": t.time,
            "price": t.price,
            "size": t.size,
        }
        for t in trades
    ]
    df = pl.DataFrame(rows).sort("t")
    every = f"{int(interval.total_seconds())}s"
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
