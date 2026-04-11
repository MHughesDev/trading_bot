"""Dependency wiring for market_data_service (scaffold publisher to Redis)."""

from __future__ import annotations

from fastapi import FastAPI

from services.common import build_scaffold_app
from shared.messaging import topics
from shared.messaging.envelope import EventEnvelope
from shared.messaging.factory import create_message_bus
from shared.messaging.redis_streams import RedisStreamsMessageBus
from shared.messaging.trace import new_trace_id


def create_app() -> FastAPI:
    bus = create_message_bus()
    app = build_scaffold_app("market_data_service")

    @app.post("/ingest/raw-tick")
    def ingest_raw_tick(payload: dict) -> dict[str, bool | str]:
        """Publish a normalized tick envelope (scaffold — not Kraken WS)."""
        symbol = str(payload.get("symbol", "BTC-USD"))
        mid = float(payload.get("mid_price", payload.get("price", 50_000.0)))
        env = EventEnvelope(
            event_type="market.tick.normalized",
            event_version="v1",
            trace_id=str(payload.get("trace_id", new_trace_id())),
            producer_service="market_data_service",
            symbol=symbol,
            payload={
                "symbol": symbol,
                "mid_price": mid,
                "price": mid,
                "direction": int(payload.get("direction", 1)),
                "size_fraction": float(payload.get("size_fraction", 0.1)),
                "route_id": str(payload.get("route_id", "SCALPING")),
                "spread_bps": float(payload.get("spread_bps", 5.0)),
            },
        )
        bus.publish(topics.MARKET_TICK_NORMALIZED_V1, env)
        return {"published": True, "topic": topics.MARKET_TICK_NORMALIZED_V1}

    @app.get("/messaging")
    def messaging_info() -> dict[str, str]:
        backend = "redis_streams" if isinstance(bus, RedisStreamsMessageBus) else "in_memory"
        return {"messaging_backend": backend}

    return app
