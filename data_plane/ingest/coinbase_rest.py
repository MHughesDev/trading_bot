from __future__ import annotations

import logging
from datetime import UTC, datetime
from typing import Any

import aiohttp

from app.config.settings import MarketDataSettings
from app.contracts.events import BarEvent

logger = logging.getLogger(__name__)


class CoinbaseRestClient:
    """Thin Coinbase REST client for candles/products metadata."""

    def __init__(self, settings: MarketDataSettings) -> None:
        self._base_url = settings.rest_url.rstrip("/")

    async def fetch_candles(
        self,
        product_id: str,
        granularity_seconds: int = 60,
        limit: int = 300,
    ) -> list[BarEvent]:
        """
        Fetch candles from Coinbase Exchange endpoint.

        Note:
        Coinbase REST returns arrays in [time, low, high, open, close, volume].
        """
        url = f"{self._base_url}/products/{product_id}/candles"
        params = {
            "granularity": granularity_seconds,
        }
        async with aiohttp.ClientSession() as session:
            async with session.get(url, params=params, timeout=20) as resp:
                resp.raise_for_status()
                payload: list[list[float]] = await resp.json()

        payload = payload[:limit]
        bars: list[BarEvent] = []
        for row in payload:
            if len(row) < 6:
                continue
            ts, low, high, open_, close, volume = row[:6]
            bars.append(
                BarEvent(
                    symbol=product_id,
                    timestamp=datetime.fromtimestamp(float(ts), tz=UTC),
                    open=float(open_),
                    high=float(high),
                    low=float(low),
                    close=float(close),
                    volume=float(volume),
                )
            )

        logger.info(
            "coinbase_rest_candles_fetched",
            extra={
                "product_id": product_id,
                "count": len(bars),
                "granularity": granularity_seconds,
            },
        )
        return sorted(bars, key=lambda b: b.timestamp)

    async def fetch_products(self) -> list[dict[str, Any]]:
        url = f"{self._base_url}/products"
        async with aiohttp.ClientSession() as session:
            async with session.get(url, timeout=20) as resp:
                resp.raise_for_status()
                payload: list[dict[str, Any]] = await resp.json()
        return payload
