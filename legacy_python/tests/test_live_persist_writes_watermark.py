"""Phase B: persisting a closed bar must advance the watermark sidecar."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta

import app.runtime.canonical_bar_watermark as _wm_mod
from app.runtime.canonical_bar_watermark import read_canonical_through, write_canonical_through
from pathlib import Path


def test_watermark_advances_after_bar_persist(tmp_path, monkeypatch) -> None:
    """Simulates what live_service does when it persists a completed bar."""
    monkeypatch.setattr(_wm_mod, "_DEFAULT_DIR", tmp_path)

    symbol = "BTC-USD"
    interval = 60
    bar_ts = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)

    # Before any write: no watermark
    assert read_canonical_through(symbol) is None

    # Simulate the live_service persist block writing the watermark
    write_canonical_through(symbol, canonical_through_ts=bar_ts, interval_seconds=interval)

    wm = read_canonical_through(symbol)
    assert wm is not None
    assert abs((wm - bar_ts).total_seconds()) < 1

    # Simulate a later bar advancing the watermark
    later_ts = bar_ts + timedelta(seconds=interval * 3)
    write_canonical_through(symbol, canonical_through_ts=later_ts, interval_seconds=interval)

    wm2 = read_canonical_through(symbol)
    assert wm2 is not None
    assert abs((wm2 - later_ts).total_seconds()) < 1
    assert wm2 > wm
