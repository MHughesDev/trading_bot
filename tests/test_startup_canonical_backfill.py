"""FB-AP-019: startup Kraken backfill orchestration."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from unittest.mock import AsyncMock, patch

import polars as pl
import pytest

from app.config.settings import AppSettings
from app.runtime import canonical_bar_watermark as wm
from data_plane.storage.startup_gap_detection import CanonicalBarGap
from orchestration.startup_canonical_backfill import backfill_gap_to_questdb


@pytest.mark.asyncio
async def test_backfill_inserts_bars(monkeypatch: pytest.MonkeyPatch, tmp_path) -> None:
    monkeypatch.setattr(wm, "_DEFAULT_DIR", tmp_path)
    bar_sec = 60
    last_closed = datetime(2026, 4, 12, 11, 59, 0, tzinfo=UTC)
    gap = CanonicalBarGap(
        symbol="BTC-USD",
        interval_seconds=bar_sec,
        max_stored_ts=last_closed - timedelta(seconds=bar_sec),
        wall_clock_utc=datetime(2026, 4, 12, 12, 0, 30, tzinfo=UTC),
        last_closed_bucket_start=last_closed,
        gap_detected=True,
        gap_start=last_closed,
        gap_end=last_closed,
        behind_seconds=60.0,
    )
    df = pl.DataFrame(
        {
            "timestamp": [last_closed],
            "open": [100.0],
            "high": [101.0],
            "low": [99.0],
            "close": [100.5],
            "volume": [1.0],
        }
    )
    with patch(
        "orchestration.startup_canonical_backfill.fetch_symbol_bars_async",
        new_callable=AsyncMock,
        return_value=df,
    ):
        qdb = AsyncMock()
        cfg = AppSettings()
        summary = await backfill_gap_to_questdb(cfg, qdb, gap, max_lookback_days=14)
    assert summary.get("inserted") == 1
    qdb.insert_bar.assert_awaited()
