"""Dependency wiring for feature_service."""

from __future__ import annotations

from dataclasses import dataclass, field

from fastapi import FastAPI

from services.common import build_scaffold_app
from services.feature_service.handlers import FeatureServiceHandlers
from services.redis_poll_lifespan import redis_stream_poll_lifespan
from shared.messaging import topics
from shared.messaging.envelope import EventEnvelope
from shared.messaging.factory import create_message_bus
from shared.messaging.redis_streams import RedisStreamsMessageBus


@dataclass
class FeatureServiceState:
    handler: FeatureServiceHandlers
    feature_rows: list[dict] = field(default_factory=list)


def create_app() -> FastAPI:
    bus = create_message_bus()
    handler = FeatureServiceHandlers(bus)
    handler.register()

    state = FeatureServiceState(handler=handler)

    def _capture_features(env: EventEnvelope) -> None:
        state.feature_rows.append(env.payload)

    bus.subscribe(topics.FEATURES_ROW_GENERATED_V1, _capture_features)

    lifespan = redis_stream_poll_lifespan(bus, [topics.MARKET_TICK_NORMALIZED_V1])
    app = build_scaffold_app("feature_service", lifespan=lifespan)

    @app.post("/ingest/market-tick")
    def ingest_market_tick(payload: dict) -> dict[str, int]:
        env = EventEnvelope(
            event_type="market.tick.normalized",
            trace_id=str(payload.get("trace_id", "manual")),
            producer_service="market_data_service",
            symbol=payload.get("symbol"),
            payload=payload,
        )
        bus.publish(topics.MARKET_TICK_NORMALIZED_V1, env)
        if isinstance(bus, RedisStreamsMessageBus):
            bus.poll_once(topics.MARKET_TICK_NORMALIZED_V1)
        return {"feature_rows": len(state.feature_rows)}

    @app.get("/events/recent")
    def events_recent() -> dict[str, list[dict]]:
        return {"feature_rows": list(state.feature_rows)}

    @app.get("/messaging")
    def messaging_info() -> dict[str, str]:
        backend = "redis_streams" if isinstance(bus, RedisStreamsMessageBus) else "in_memory"
        return {"messaging_backend": backend}

    return app
