from __future__ import annotations

import asyncio
import json
import logging
from collections.abc import Awaitable, Callable
from datetime import UTC, datetime
from typing import Any

import websockets

from app.config.settings import MarketDataSettings
from app.contracts.events import BarEvent, OrderBookEvent, TickerEvent, TradeEvent
from data_plane.ingest.normalizers import (
    normalize_bar,
    normalize_l2,
    normalize_ticker,
    normalize_trade,
)

logger = logging.getLogger(__name__)

EventHandler = Callable[[TickerEvent | TradeEvent | BarEvent | OrderBookEvent], Awaitable[None]]


class CoinbaseWebSocketIngest:
    """
    Coinbase WS market data ingest service.

    Non-negotiable behavior:
    - Coinbase is the only market data source.
    - All outbound events are normalized typed contracts.
    """

    def __init__(
        self,
        settings: MarketDataSettings,
        event_handler: EventHandler,
    ) -> None:
        self._settings = settings
        self._event_handler = event_handler
        self._url = settings.websocket_url
        self._symbols = settings.symbols
        self._channels = settings.channels
        self._ws: Any = None
        self._running = False

    async def _subscribe(self) -> None:
        if self._ws is None:
            raise RuntimeError("WebSocket connection not established")
        payload = {
            "type": "subscribe",
            "product_ids": self._symbols,
            "channels": self._channels,
        }
        await self._ws.send(json.dumps(payload))
        logger.info("coinbase_ws_subscribed", extra={"payload": payload})

    async def _handle_message(self, raw_msg: str) -> None:
        msg = json.loads(raw_msg)
        channel = msg.get("channel") or msg.get("type")

        try:
            if channel == "ticker":
                evt = normalize_ticker(msg)
                await self._event_handler(evt)
            elif channel in {"market_trades", "match"}:
                evt = normalize_trade(msg)
                await self._event_handler(evt)
            elif channel in {"candles", "candle"}:
                evt = normalize_bar(msg)
                await self._event_handler(evt)
            elif channel == "level2":
                evt = normalize_l2(msg)
                await self._event_handler(evt)
            elif msg.get("type") in {"subscriptions", "heartbeat"}:
                logger.debug("coinbase_ws_control_message", extra={"msg": msg})
            else:
                logger.debug("coinbase_ws_unhandled_message", extra={"msg": msg})
        except Exception:
            logger.exception("coinbase_ws_normalization_error", extra={"msg": msg})

    async def run(self) -> None:
        self._running = True
        backoff_seconds = 1.0

        while self._running:
            try:
                async with websockets.connect(self._url, ping_interval=20, ping_timeout=20) as ws:
                    self._ws = ws
                    await self._subscribe()
                    backoff_seconds = 1.0
                    logger.info("coinbase_ws_connected", extra={"url": self._url})

                    async for raw_msg in ws:
                        await self._handle_message(raw_msg)
            except asyncio.CancelledError:
                raise
            except Exception:
                logger.exception("coinbase_ws_connection_error")
                await asyncio.sleep(backoff_seconds)
                backoff_seconds = min(backoff_seconds * 2, 30.0)
            finally:
                self._ws = None

    async def stop(self) -> None:
        self._running = False
        if self._ws is not None:
            await self._ws.close()
            self._ws = None


async def print_event_handler(
    event: TickerEvent | TradeEvent | BarEvent | OrderBookEvent,
) -> None:
    logger.info(
        "market_event",
        extra={
            "event_type": type(event).__name__,
            "symbol": getattr(event, "symbol", ""),
            "timestamp": getattr(event, "timestamp", datetime.now(UTC)).isoformat(),
        },
    )
