"""Phase C: RollingBars.seed() warms up the roller from historical rows."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta

import polars as pl

from data_plane.bars.rolling import RollingBars


def _make_rows(n: int, base_ts: datetime, interval: int = 60, base_price: float = 100.0):
    rows = []
    for i in range(n):
        ts = base_ts + timedelta(seconds=interval * i)
        p = base_price + i * 0.1
        rows.append({
            "timestamp": ts,
            "open": p * 0.999,
            "high": p * 1.001,
            "low": p * 0.998,
            "close": p,
            "volume": 1000.0 + i,
        })
    return rows


def test_seed_populates_completed_bars() -> None:
    base = datetime(2026, 1, 1, 10, 0, 0, tzinfo=UTC)
    rows = _make_rows(10, base)
    roller = RollingBars("BTC-USD", interval_seconds=60, max_completed=512)
    roller.seed(rows)

    completed = roller.bars_frame_completed()
    assert completed.height == 10
    assert list(completed["timestamp"]) == [r["timestamp"] for r in rows]


def test_seed_respects_max_completed() -> None:
    base = datetime(2026, 1, 1, 0, 0, 0, tzinfo=UTC)
    rows = _make_rows(20, base)
    roller = RollingBars("BTC-USD", interval_seconds=60, max_completed=8)
    roller.seed(rows)

    completed = roller.bars_frame_completed()
    assert completed.height == 8  # tail 8 kept
    # Last timestamp should be from the last row
    assert completed["timestamp"][-1] == rows[-1]["timestamp"]


def test_on_tick_continues_after_seed() -> None:
    """First on_tick after seed should produce a completed bar in the next interval."""
    base = datetime(2026, 1, 1, 10, 0, 0, tzinfo=UTC)
    rows = _make_rows(5, base)
    roller = RollingBars("BTC-USD", interval_seconds=60, max_completed=512)
    roller.seed(rows)

    # Tick that starts a new bucket (6th minute)
    new_ts = base + timedelta(seconds=60 * 5)
    result = roller.on_tick(105.0, new_ts, 500.0)
    assert result is None  # opens new bucket, not closed yet

    # Tick in the 7th minute — should close the 6th-minute bucket
    next_ts = base + timedelta(seconds=60 * 6)
    completed = roller.on_tick(106.0, next_ts, 600.0)
    assert completed is not None
    # The completed bar is the 6th-minute bucket
    assert completed["close"] == 105.0


def test_seed_from_polars_dataframe() -> None:
    base = datetime(2026, 1, 1, 8, 0, 0, tzinfo=UTC)
    rows = _make_rows(5, base)
    df = pl.DataFrame(rows)
    roller = RollingBars("ETH-USD", interval_seconds=60)
    roller.seed(df)
    assert roller.bars_frame_completed().height == 5


def test_seed_empty_is_noop() -> None:
    roller = RollingBars("BTC-USD", interval_seconds=60)
    roller.seed([])
    assert roller.bars_frame_completed().height == 0
    assert roller._bucket_start is None
