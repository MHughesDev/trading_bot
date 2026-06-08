"""FB-AP-019: canonical bar watermark sidecar."""

from __future__ import annotations

from datetime import UTC, datetime

import pytest

from app.runtime import canonical_bar_watermark as wm


def test_read_write_watermark(tmp_path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(wm, "_DEFAULT_DIR", tmp_path)
    ts = datetime(2026, 4, 12, 12, 0, 0, tzinfo=UTC)
    wm.write_canonical_through("BTC-USD", canonical_through_ts=ts, interval_seconds=60)
    assert wm.read_canonical_through("BTC-USD") == ts


def test_read_missing(tmp_path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(wm, "_DEFAULT_DIR", tmp_path)
    assert wm.read_canonical_through("ETH-USD") is None
