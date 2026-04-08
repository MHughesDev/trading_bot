"""Coinbase REST: Advanced Trade 401 → Exchange public fallback (Issue 35)."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from unittest.mock import AsyncMock, MagicMock

import pytest

from data_plane.ingest.coinbase_rest import CoinbaseRESTClient


@pytest.mark.asyncio
async def test_get_public_candles_falls_back_on_401(monkeypatch) -> None:
    end = datetime.now(UTC)
    start = end - timedelta(hours=2)

    adv401 = MagicMock()
    adv401.status_code = 401
    adv401.request = MagicMock()

    exchange_ok = MagicMock()
    exchange_ok.status_code = 200
    exchange_ok.json.return_value = [
        [int(start.timestamp()), 100.0, 101.0, 100.5, 100.75, 12.0],
    ]
    exchange_ok.raise_for_status = MagicMock()

    client = CoinbaseRESTClient()
    monkeypatch.setattr(client._client, "request", AsyncMock(return_value=adv401))
    monkeypatch.setattr(client._exchange, "request", AsyncMock(return_value=exchange_ok))

    out = await client.get_public_candles("BTC-USD", start=start, end=end, granularity_seconds=3600)
    await client.aclose()

    assert len(out) == 1
    assert out[0].close == 100.75


@pytest.mark.asyncio
async def test_list_products_falls_back_on_403(monkeypatch) -> None:
    adv403 = MagicMock()
    adv403.status_code = 403
    adv403.request = MagicMock()

    exchange_ok = MagicMock()
    exchange_ok.status_code = 200
    exchange_ok.json.return_value = [{"id": "BTC-USD", "base_currency": "BTC", "quote_currency": "USD"}]
    exchange_ok.raise_for_status = MagicMock()

    client = CoinbaseRESTClient()
    monkeypatch.setattr(client._client, "request", AsyncMock(return_value=adv403))
    monkeypatch.setattr(client._exchange, "request", AsyncMock(return_value=exchange_ok))

    products = await client.list_products()
    await client.aclose()

    assert len(products) == 1
    assert products[0].product_id == "BTC-USD"
