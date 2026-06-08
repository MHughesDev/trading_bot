"""Phase B: startup gap detection reads the watermark to bound the dark period."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta

import app.runtime.canonical_bar_watermark as _wm_mod
from app.runtime.canonical_bar_watermark import write_canonical_through
from data_plane.storage.startup_gap_detection import last_closed_bucket_start_utc


def test_gap_window_bounded_by_watermark(tmp_path, monkeypatch) -> None:
    """After writing a watermark, the gap window starts at watermark + interval."""
    monkeypatch.setattr(_wm_mod, "_DEFAULT_DIR", tmp_path)

    interval = 60
    now = datetime.now(UTC)
    # Write a watermark 5 bars behind the current last-closed bucket.
    last_closed = last_closed_bucket_start_utc(now, interval)
    wm_ts = last_closed - timedelta(seconds=interval * 5)
    write_canonical_through("BTC-USD", canonical_through_ts=wm_ts, interval_seconds=interval)

    # Re-read and verify
    from app.runtime.canonical_bar_watermark import read_canonical_through

    read_back = read_canonical_through("BTC-USD")
    assert read_back is not None
    assert abs((read_back - wm_ts).total_seconds()) < 1  # round-trip through ISO is exact

    # The expected gap window: [watermark + interval, last_closed]
    gap_start = read_back + timedelta(seconds=interval)
    gap_bars = int((last_closed - read_back).total_seconds() / interval)
    assert gap_bars == 5
    assert gap_start <= last_closed


def test_no_watermark_returns_none(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(_wm_mod, "_DEFAULT_DIR", tmp_path)
    from app.runtime.canonical_bar_watermark import read_canonical_through

    assert read_canonical_through("ETH-USD") is None
