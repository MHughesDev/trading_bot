"""FB-AP-018: startup gap detection for canonical bars."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from unittest.mock import AsyncMock

import pytest

from data_plane.storage.startup_gap_detection import (
    detect_canonical_bar_gaps,
    last_closed_bucket_start_utc,
)


def test_last_closed_bucket_start_utc() -> None:
    # 12:00:30 UTC, 60s bars → last closed bucket starts 11:59:00
    now = datetime(2026, 4, 12, 12, 0, 30, tzinfo=UTC)
    assert last_closed_bucket_start_utc(now, 60) == datetime(
        2026, 4, 12, 11, 59, 0, tzinfo=UTC
    )
    # 12:00:00.5 → still in [12:00:00, 12:01:00) → last closed 11:59:00
    now2 = datetime(2026, 4, 12, 12, 0, 0, 500000, tzinfo=UTC)
    assert last_closed_bucket_start_utc(now2, 60) == datetime(
        2026, 4, 12, 11, 59, 0, tzinfo=UTC
    )


@pytest.mark.asyncio
async def test_detect_gaps_no_rows() -> None:
    qdb = AsyncMock()
    qdb.max_canonical_bar_timestamp = AsyncMock(return_value=None)
    now = datetime(2026, 4, 12, 12, 0, 30, tzinfo=UTC)
    gaps = await detect_canonical_bar_gaps(
        qdb,
        symbols=["BTC-USD"],
        interval_seconds=60,
        wall_clock_utc=now,
    )
    assert len(gaps) == 1
    g = gaps[0]
    assert g.gap_detected is True
    assert g.gap_start is None
    assert g.behind_seconds is None
    qdb.max_canonical_bar_timestamp.assert_awaited_once_with(
        "BTC-USD", interval_seconds=60
    )


@pytest.mark.asyncio
async def test_detect_gaps_caught_up() -> None:
    qdb = AsyncMock()
    last_closed = last_closed_bucket_start_utc(
        datetime(2026, 4, 12, 12, 0, 30, tzinfo=UTC), 60
    )
    qdb.max_canonical_bar_timestamp = AsyncMock(return_value=last_closed)
    gaps = await detect_canonical_bar_gaps(
        qdb,
        symbols=["BTC-USD"],
        interval_seconds=60,
        wall_clock_utc=datetime(2026, 4, 12, 12, 0, 30, tzinfo=UTC),
    )
    assert gaps[0].gap_detected is False
    assert gaps[0].behind_seconds == 0.0


@pytest.mark.asyncio
async def test_detect_gaps_behind() -> None:
    qdb = AsyncMock()
    # Stored max is one full bar before last_closed
    wall = datetime(2026, 4, 12, 12, 0, 30, tzinfo=UTC)
    last_closed = last_closed_bucket_start_utc(wall, 60)
    stale = last_closed - timedelta(seconds=60)
    qdb.max_canonical_bar_timestamp = AsyncMock(return_value=stale)
    gaps = await detect_canonical_bar_gaps(
        qdb,
        symbols=["ETH-USD"],
        interval_seconds=60,
        wall_clock_utc=wall,
    )
    g = gaps[0]
    assert g.gap_detected is True
    assert g.gap_start == last_closed
    assert g.behind_seconds == pytest.approx(60.0)
