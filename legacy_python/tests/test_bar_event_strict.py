"""Strict BarEvent validation (FB-F4)."""

from __future__ import annotations

from datetime import UTC, datetime

import pytest

from app.contracts.events import BarEvent


def test_bar_event_rejects_inverted_ohlc() -> None:
    ts = datetime.now(UTC)
    with pytest.raises(ValueError):
        BarEvent(
            timestamp=ts,
            symbol="BTC-USD",
            open=1.0,
            high=1.0,
            low=2.0,
            close=1.5,
            volume=1.0,
        )
