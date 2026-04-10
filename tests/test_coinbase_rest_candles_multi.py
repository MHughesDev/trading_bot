"""REST candles for multiple products (FB-F3)."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from unittest.mock import AsyncMock, MagicMock

import pytest

from data_plane.ingest.coinbase_rest import CoinbaseRESTClient


def _ok_candle_json() -> list:
    end = datetime.now(UTC)
    start = end - timedelta(hours=1)
    return [[int(start.timestamp()), 100.0, 102.0, 101.0, 101.5, 10.0]]


@pytest.mark.asyncio
@pytest.mark.parametrize("product_id", ["BTC-USD", "ETH-USD", "SOL-USD"])
async def test_get_public_candles_exchange_path(product_id: str, monkeypatch) -> None:
    end = datetime.now(UTC)
    start = end - timedelta(hours=2)

    adv401 = MagicMock()
    adv401.status_code = 401
    adv401.request = MagicMock()

    exchange_ok = MagicMock()
    exchange_ok.status_code = 200
    exchange_ok.json.return_value = _ok_candle_json()
    exchange_ok.raise_for_status = MagicMock()

    client = CoinbaseRESTClient()
    monkeypatch.setattr(client._client, "request", AsyncMock(return_value=adv401))
    monkeypatch.setattr(client._exchange, "request", AsyncMock(return_value=exchange_ok))

    out = await client.get_public_candles(product_id, start=start, end=end, granularity_seconds=3600)
    await client.aclose()

    assert len(out) == 1
    assert out[0].close == 101.5
