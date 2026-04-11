"""Kraken public REST client — OHLC parse and symbol mapping."""

from __future__ import annotations

from datetime import UTC, datetime
from unittest.mock import AsyncMock, MagicMock

import pytest

from data_plane.ingest.kraken_rest import KrakenRESTClient, granularity_to_kraken_ohlc_interval_minutes
from data_plane.ingest.kraken_symbols import kraken_rest_pair, kraken_ws_pair


def test_granularity_to_interval_minutes() -> None:
    assert granularity_to_kraken_ohlc_interval_minutes(60) == 1
    assert granularity_to_kraken_ohlc_interval_minutes(300) == 5
    assert granularity_to_kraken_ohlc_interval_minutes(45) is None


def test_symbol_mapping() -> None:
    assert kraken_rest_pair("BTC-USD") == "XBTUSD"
    assert kraken_ws_pair("BTC-USD") == "XBT/USD"


@pytest.mark.asyncio
async def test_ohlc_parses_rows(monkeypatch) -> None:
    body = {
        "error": [],
        "result": {
            "XXBTZUSD": [
                [1700000000, "100.0", "101.0", "99.0", "100.5", "100.2", "12.0", 10],
            ],
            "last": 1700000000,
        },
    }
    ok = MagicMock()
    ok.status_code = 200
    ok.json.return_value = body
    ok.raise_for_status = MagicMock()

    client = KrakenRESTClient()
    monkeypatch.setattr(client._client, "get", AsyncMock(return_value=ok))

    rows, last = await client.ohlc("XBTUSD", interval_minutes=1)
    await client.aclose()

    assert len(rows) == 1
    assert rows[0].close == 100.5
    assert rows[0].volume == 12.0
    assert last == 1700000000


@pytest.mark.asyncio
async def test_fetch_ohlc_range_filters_window(monkeypatch) -> None:
    from data_plane.ingest.kraken_rest import fetch_ohlc_range

    t0 = 1700000000
    calls = {"n": 0}

    async def fake_ohlc(self, pair, *, interval_minutes, since=None):
        calls["n"] += 1
        from data_plane.ingest.kraken_rest import KrakenOHLCRow

        if calls["n"] == 1:
            return (
                [
                    KrakenOHLCRow(time=t0, open=1, high=2, low=0.5, close=1.5, volume=1),
                    KrakenOHLCRow(time=t0 + 60, open=1.5, high=2, low=1, close=1.8, volume=2),
                ],
                t0,
            )
        return ([KrakenOHLCRow(time=t0 + 120, open=2, high=3, low=1, close=2.5, volume=1)], t0 + 120)

    # Patch instance method
    client = KrakenRESTClient()
    monkeypatch.setattr(KrakenRESTClient, "ohlc", fake_ohlc)

    start = datetime.fromtimestamp(t0, tz=UTC)
    end = datetime.fromtimestamp(t0 + 30, tz=UTC)
    rows = await fetch_ohlc_range(client, "XBTUSD", start, end, interval_minutes=1)
    await client.aclose()

    assert len(rows) == 1
    assert rows[0].time == t0
