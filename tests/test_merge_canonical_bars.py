"""FB-AP-015: idempotent merge / dedupe for canonical bar frames."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from unittest.mock import AsyncMock, MagicMock

import polars as pl
import pytest

from app.contracts.events import BarEvent
from data_plane.storage.merge_canonical_bars import dedupe_canonical_bars_last_wins, merge_canonical_bars_frames
from data_plane.storage.questdb import QuestDBWriter


def _row(
    ts: datetime,
    sym: str,
    interval: int,
    close: float,
) -> dict:
    return {
        "timestamp": ts,
        "symbol": sym,
        "interval_seconds": interval,
        "open": 1.0,
        "high": 1.0,
        "low": 1.0,
        "close": close,
        "volume": 1.0,
    }


def test_dedupe_last_wins_same_key() -> None:
    t = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)
    df = pl.DataFrame(
        [
            _row(t, "BTC-USD", 60, 1.0),
            _row(t, "BTC-USD", 60, 2.0),
        ]
    )
    out = dedupe_canonical_bars_last_wins(df)
    assert out.height == 1
    assert float(out["close"][0]) == 2.0


def test_merge_two_frames_dedupes() -> None:
    t0 = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)
    t1 = t0 + timedelta(minutes=1)
    a = pl.DataFrame([_row(t0, "BTC-USD", 60, 1.0)])
    b = pl.DataFrame(
        [
            _row(t0, "BTC-USD", 60, 99.0),
            _row(t1, "BTC-USD", 60, 3.0),
        ]
    )
    out = merge_canonical_bars_frames(a, b)
    assert out.height == 2
    t0_row = out.filter(pl.col("timestamp") == t0)
    assert float(t0_row["close"][0]) == 99.0


def test_dedupe_missing_column_raises() -> None:
    df = pl.DataFrame({"timestamp": [datetime.now(UTC)], "symbol": ["X"]})
    with pytest.raises(ValueError, match="missing columns"):
        dedupe_canonical_bars_last_wins(df)


@pytest.mark.asyncio
async def test_insert_bar_deletes_then_inserts() -> None:
    w = QuestDBWriter("h", 8812, "u", "p")
    w._conn = MagicMock()
    w._conn.execute = AsyncMock()

    ts = datetime(2026, 1, 1, tzinfo=UTC)
    bar = BarEvent(
        timestamp=ts,
        symbol="BTC-USD",
        open=1.0,
        high=1.0,
        low=1.0,
        close=1.0,
        volume=1.0,
        interval_seconds=60,
    )
    await w.insert_bar(bar)
    assert w._conn.execute.await_count == 2
    calls = w._conn.execute.await_args_list
    assert "DELETE FROM canonical_bars" in str(calls[0][0][0])
    assert calls[0][0][1] == ("BTC-USD", ts, 60)
    assert "INSERT INTO canonical_bars" in str(calls[1][0][0])
