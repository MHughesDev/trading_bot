"""Unit tests for market_data_service Kraken → envelope helpers."""

from __future__ import annotations

from datetime import UTC, datetime

from data_plane.ingest.normalizers import TickerSnapshot
from services.market_data_service.kraken_ticks import heartbeat_envelope, ticker_to_normalized_tick_envelope


def test_ticker_to_envelope_uses_mid_and_spread() -> None:
    ts = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)
    snap = TickerSnapshot(
        symbol="XBT/USD",
        price=100_000.0,
        time=ts,
        bid=99_000.0,
        ask=101_000.0,
        raw={},
    )
    env = ticker_to_normalized_tick_envelope(snap)
    assert env.payload["symbol"] == "XBT/USD"
    assert env.payload["mid_price"] == 100_000.0
    assert env.payload["spread_bps"] > 0
    assert "data_timestamp" in env.payload


def test_heartbeat_envelope() -> None:
    env = heartbeat_envelope(["BTC-USD"], last_tick_at=None)
    assert env.event_type == "market.heartbeat"
    assert "BTC-USD" in env.payload["symbols"]
