"""
Kraken WebSocket v1 — public market data (ticker, trade, book).

Docs: https://docs.kraken.com/api/docs/websocket-v1/book
"""

from __future__ import annotations

import asyncio
import json
import logging
from collections.abc import AsyncIterator
from datetime import UTC, datetime
from typing import Any

import websockets
from pydantic_settings import BaseSettings, SettingsConfigDict

from data_plane.ingest.kraken_symbols import kraken_ws_pair

logger = logging.getLogger(__name__)

KRAKEN_WS_V1_URL = "wss://ws.kraken.com"


class KrakenWSSettings(BaseSettings):
    model_config = SettingsConfigDict(env_prefix="KRAKEN_WS_", env_file=".env", extra="ignore")

    url: str = KRAKEN_WS_V1_URL
    reconnect_initial_seconds: float = 1.0
    reconnect_max_seconds: float = 60.0
    ping_interval: float | None = 20.0
    ping_timeout: float | None = 20.0
    open_timeout: float = 30.0


class KrakenWebSocketClient:
    """
    Kraken WS v1: subscribe to ticker + trade for each pair (Kraken ``XBT/USD`` style).
    Yields raw list/dict messages; use ``normalize_kraken_ws_message``.
    """

    def __init__(
        self,
        symbols: list[str],
        *,
        settings: KrakenWSSettings | None = None,
    ) -> None:
        if not symbols:
            raise ValueError("symbols must be non-empty")
        self._pairs = [kraken_ws_pair(s) for s in symbols]
        self._settings = settings or KrakenWSSettings()
        self._last_message_at: datetime | None = None
        self._message_count: int = 0

    @property
    def last_message_at(self) -> datetime | None:
        return self._last_message_at

    @property
    def message_count(self) -> int:
        return self._message_count

    def _subscribe_payloads(self) -> list[dict[str, Any]]:
        out: list[dict[str, Any]] = []
        for pair in self._pairs:
            out.append(
                {
                    "event": "subscribe",
                    "pair": [pair],
                    "subscription": {"name": "ticker"},
                }
            )
            out.append(
                {
                    "event": "subscribe",
                    "pair": [pair],
                    "subscription": {"name": "trade"},
                }
            )
        return out

    async def stream_messages(self) -> AsyncIterator[dict[str, Any]]:
        backoff = self._settings.reconnect_initial_seconds
        while True:
            try:
                async with websockets.connect(
                    self._settings.url,
                    ping_interval=self._settings.ping_interval,
                    ping_timeout=self._settings.ping_timeout,
                    open_timeout=self._settings.open_timeout,
                ) as ws:
                    for sub in self._subscribe_payloads():
                        await ws.send(json.dumps(sub))
                    backoff = self._settings.reconnect_initial_seconds
                    async for raw in ws:
                        self._last_message_at = datetime.now(UTC)
                        self._message_count += 1
                        try:
                            msg = json.loads(raw)
                        except json.JSONDecodeError:
                            continue
                        if isinstance(msg, dict):
                            yield msg
                        elif isinstance(msg, list) and len(msg) >= 4:
                            yield {"kraken_v1_array": True, "payload": msg}
                        elif isinstance(msg, list):
                            yield {"kraken_v1_array": True, "payload": msg}
            except Exception as e:
                logger.warning("Kraken WS reconnect after %s", e)
                await asyncio.sleep(backoff)
                backoff = min(backoff * 2, self._settings.reconnect_max_seconds)
