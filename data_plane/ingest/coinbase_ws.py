"""
Coinbase Advanced Trade WebSocket client (market data ONLY).

Spec: Coinbase is the single source of truth for market data; Alpaca is never used for data.
Docs: https://docs.cdp.coinbase.com/advanced-trade/docs/ws-overview
"""

from __future__ import annotations

import asyncio
import json
import logging
from collections.abc import AsyncIterator
from enum import StrEnum
from typing import Any

import websockets
from pydantic_settings import BaseSettings, SettingsConfigDict

logger = logging.getLogger(__name__)

COINBASE_ADVANCED_WS_URL = "wss://advanced-trade-ws.coinbase.com"


class CoinbaseChannel(StrEnum):
    HEARTBEATS = "heartbeats"
    TICKER = "ticker"
    LEVEL2 = "level2"
    MARKET_TRADES = "market_trades"
    CANDLES = "candles"


class CoinbaseWSSettings(BaseSettings):
    """Environment-driven defaults for the WS client."""

    model_config = SettingsConfigDict(
        env_prefix="COINBASE_WS_",
        env_file=".env",
        extra="ignore",
    )

    url: str = COINBASE_ADVANCED_WS_URL
    reconnect_initial_seconds: float = 1.0
    reconnect_max_seconds: float = 60.0
    ping_interval: float | None = 20.0
    ping_timeout: float | None = 20.0
    open_timeout: float = 30.0


def _subscribe_message(
    channel: CoinbaseChannel,
    product_ids: list[str],
    *,
    jwt: str | None = None,
) -> dict[str, Any]:
    msg: dict[str, Any] = {
        "type": "subscribe",
        "product_ids": product_ids,
        "channel": channel.value,
    }
    if jwt:
        msg["jwt"] = jwt
    return msg


class CoinbaseWebSocketClient:
    """
    Streams raw Coinbase Advanced Trade WebSocket messages as parsed JSON dicts.

    Authentication: public channels do not require JWT; private/user channels need CDP API JWT.
    """

    def __init__(
        self,
        product_ids: list[str],
        channels: list[CoinbaseChannel] | None = None,
        *,
        settings: CoinbaseWSSettings | None = None,
        jwt: str | None = None,
    ) -> None:
        if not product_ids:
            raise ValueError("product_ids must be non-empty")
        self._product_ids = list(product_ids)
        self._channels = channels or [
            CoinbaseChannel.HEARTBEATS,
            CoinbaseChannel.TICKER,
            CoinbaseChannel.LEVEL2,
            CoinbaseChannel.MARKET_TRADES,
        ]
        self._settings = settings or CoinbaseWSSettings()
        self._jwt = jwt

    @property
    def product_ids(self) -> list[str]:
        return list(self._product_ids)

    def subscribe_payloads(self) -> list[dict[str, Any]]:
        return [
            _subscribe_message(ch, self._product_ids, jwt=self._jwt) for ch in self._channels
        ]

    async def stream_messages(self) -> AsyncIterator[dict[str, Any]]:
        """
        Connect, subscribe, and yield each top-level JSON message forever (with reconnect).
        """
        backoff = self._settings.reconnect_initial_seconds
        while True:
            try:
                async for msg in self._connected_stream():
                    yield msg
                backoff = self._settings.reconnect_initial_seconds
            except asyncio.CancelledError:
                raise
            except Exception:
                logger.exception("Coinbase WS disconnected; reconnecting in %.1fs", backoff)
                await asyncio.sleep(backoff)
                backoff = min(backoff * 2, self._settings.reconnect_max_seconds)

    async def _connected_stream(self) -> AsyncIterator[dict[str, Any]]:
        async with websockets.connect(
            self._settings.url,
            ping_interval=self._settings.ping_interval,
            ping_timeout=self._settings.ping_timeout,
            open_timeout=self._settings.open_timeout,
            max_size=None,
        ) as ws:
            for payload in self.subscribe_payloads():
                await ws.send(json.dumps(payload))
            async for raw in ws:
                if isinstance(raw, bytes):
                    raw = raw.decode("utf-8")
                try:
                    data = json.loads(raw)
                except json.JSONDecodeError:
                    logger.warning("Non-JSON WS frame: %s", raw[:200])
                    continue
                if not isinstance(data, dict):
                    continue
                yield data


async def run_stdout_demo(product_ids: list[str] | None = None) -> None:
    """Minimal CLI demo: print first N messages (for manual smoke tests)."""
    logging.basicConfig(level=logging.INFO)
    ids = product_ids or ["BTC-USD", "ETH-USD", "SOL-USD"]
    client = CoinbaseWebSocketClient(ids)
    n = 0
    async for msg in client.stream_messages():
        print(json.dumps(msg, default=str))
        n += 1
        if n >= 5:
            break


if __name__ == "__main__":
    asyncio.run(run_stdout_demo())
