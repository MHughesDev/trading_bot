"""Dependency wiring for market_data_service (Kraken WS publisher to Redis when enabled)."""

from __future__ import annotations

import asyncio
import logging
import os
from contextlib import asynccontextmanager
from typing import Any, AsyncIterator

from fastapi import FastAPI

from data_plane.ingest.kraken_normalizers import normalize_kraken_ws_message
from data_plane.ingest.kraken_ws import KrakenWebSocketClient
from data_plane.ingest.normalizers import TickerSnapshot
from services.common import build_scaffold_app
from services.market_data_service.kraken_ticks import heartbeat_envelope, ticker_to_normalized_tick_envelope
from shared.messaging import topics
from shared.messaging.envelope import EventEnvelope
from shared.messaging.factory import create_message_bus
from shared.messaging.redis_streams import RedisStreamsMessageBus
from shared.messaging.trace import new_trace_id

logger = logging.getLogger(__name__)


def _parse_symbols() -> list[str]:
    raw = os.getenv("NM_MARKET_DATA_SYMBOLS", "BTC-USD").strip()
    if not raw:
        return ["BTC-USD"]
    return [s.strip() for s in raw.split(",") if s.strip()]


def _kraken_ws_enabled() -> bool:
    return os.getenv("NM_MARKET_DATA_KRAKEN_WS", "false").strip().lower() in (
        "1",
        "true",
        "yes",
    )


def _heartbeat_interval_s() -> float:
    try:
        return max(5.0, float(os.getenv("NM_MARKET_DATA_HEARTBEAT_SECONDS", "30")))
    except ValueError:
        return 30.0


def _publish_envelope(bus: Any, env: EventEnvelope) -> None:
    bus.publish(topics.MARKET_TICK_NORMALIZED_V1, env)
    if isinstance(bus, RedisStreamsMessageBus):
        bus.poll_once(topics.MARKET_TICK_NORMALIZED_V1)


@asynccontextmanager
async def _kraken_lifespan(bus: Any) -> AsyncIterator[None]:
    if not _kraken_ws_enabled():
        yield
        return

    symbols = _parse_symbols()
    stop = asyncio.Event()
    tick_state: dict[str, Any] = {"last_tick_at": None}

    async def _run_ws() -> None:
        client = KrakenWebSocketClient(symbols)
        try:
            async for msg in client.stream_messages():
                if stop.is_set():
                    break
                norm = normalize_kraken_ws_message(msg)
                if norm is None or not isinstance(norm, TickerSnapshot):
                    continue
                tick_state["last_tick_at"] = norm.time
                env = ticker_to_normalized_tick_envelope(norm)
                _publish_envelope(bus, env)
        except asyncio.CancelledError:
            raise
        except Exception as e:  # noqa: BLE001
            logger.exception("Kraken WS loop failed: %s", e)

    async def _heartbeat_loop() -> None:
        while not stop.is_set():
            await asyncio.sleep(_heartbeat_interval_s())
            if stop.is_set():
                break
            last_at = tick_state.get("last_tick_at")
            env = heartbeat_envelope(symbols, last_tick_at=last_at)
            bus.publish(topics.MARKET_HEARTBEAT_V1, env)
            if isinstance(bus, RedisStreamsMessageBus):
                bus.poll_once(topics.MARKET_HEARTBEAT_V1)

    ws_task = asyncio.create_task(_run_ws())
    hb_task = asyncio.create_task(_heartbeat_loop())
    try:
        yield
    finally:
        stop.set()
        ws_task.cancel()
        hb_task.cancel()
        for t in (ws_task, hb_task):
            try:
                await t
            except asyncio.CancelledError:
                pass


def create_app() -> FastAPI:
    bus = create_message_bus()
    lifespan = _kraken_lifespan(bus)
    app = build_scaffold_app("market_data_service", lifespan=lifespan)

    @app.post("/ingest/raw-tick")
    def ingest_raw_tick(payload: dict) -> dict[str, bool | str]:
        """Publish a normalized tick envelope (manual / test path when WS is off)."""
        symbol = str(payload.get("symbol", "BTC-USD"))
        mid = float(payload.get("mid_price", payload.get("price", 50_000.0)))
        env = EventEnvelope(
            event_type="market.tick.normalized",
            event_version="v1",
            trace_id=str(payload.get("trace_id", new_trace_id())),
            producer_service="market_data_service",
            symbol=symbol,
            partition_key=symbol,
            payload={
                "symbol": symbol,
                "mid_price": mid,
                "price": mid,
                "direction": int(payload.get("direction", 1)),
                "size_fraction": float(payload.get("size_fraction", 0.1)),
                "route_id": str(payload.get("route_id", "SCALPING")),
                "spread_bps": float(payload.get("spread_bps", 5.0)),
                "source": str(payload.get("source", "manual_http")),
            },
        )
        bus.publish(topics.MARKET_TICK_NORMALIZED_V1, env)
        if isinstance(bus, RedisStreamsMessageBus):
            bus.poll_once(topics.MARKET_TICK_NORMALIZED_V1)
        return {"published": True, "topic": topics.MARKET_TICK_NORMALIZED_V1}

    @app.get("/messaging")
    def messaging_info() -> dict[str, str]:
        backend = "redis_streams" if isinstance(bus, RedisStreamsMessageBus) else "in_memory"
        return {
            "messaging_backend": backend,
            "kraken_ws": "on" if _kraken_ws_enabled() else "off",
            "symbols": ",".join(_parse_symbols()),
        }

    return app
