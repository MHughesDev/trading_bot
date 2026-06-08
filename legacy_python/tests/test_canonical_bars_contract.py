"""FB-AP-014 canonical bar contract (BarEvent + Parquet column order)."""

from __future__ import annotations

from datetime import UTC, datetime

import polars as pl
import pytest

from app.contracts.events import BarEvent
from data_plane.storage.canonical_bars import CANONICAL_BAR_PARQUET_COLUMNS


def test_bar_event_default_interval_one_second() -> None:
    ts = datetime.now(UTC)
    b = BarEvent(
        timestamp=ts,
        symbol="BTC-USD",
        open=1.0,
        high=1.0,
        low=1.0,
        close=1.0,
        volume=1.0,
    )
    assert b.interval_seconds == 1


def test_bar_event_rejects_zero_interval() -> None:
    ts = datetime.now(UTC)
    with pytest.raises(ValueError):
        BarEvent(
            timestamp=ts,
            symbol="BTC-USD",
            open=1.0,
            high=1.0,
            low=1.0,
            close=1.0,
            volume=1.0,
            interval_seconds=0,
        )


def test_canonical_parquet_column_order_matches_dataframe() -> None:
    t = datetime(2026, 1, 1, tzinfo=UTC)
    df = pl.DataFrame(
        {
            "timestamp": [t],
            "symbol": ["BTC-USD"],
            "interval_seconds": [60],
            "open": [1.0],
            "high": [1.0],
            "low": [1.0],
            "close": [1.0],
            "volume": [1.0],
        }
    )
    ordered = df.select([c for c in CANONICAL_BAR_PARQUET_COLUMNS if c in df.columns])
    assert ordered.columns == list(CANONICAL_BAR_PARQUET_COLUMNS)
