"""Dependency wiring for decision service."""

from __future__ import annotations

from dataclasses import dataclass, field

from fastapi import FastAPI

from services.common import build_scaffold_app
from services.decision_service.handlers import DecisionServiceHandlers
from services.redis_poll_lifespan import redis_stream_poll_lifespan
from shared.messaging import topics
from shared.messaging.envelope import EventEnvelope
from shared.messaging.factory import create_message_bus
from shared.messaging.redis_streams import RedisStreamsMessageBus


@dataclass
class DecisionServiceState:
    handler: DecisionServiceHandlers
    proposals: list[dict] = field(default_factory=list)


def create_app() -> FastAPI:
    bus = create_message_bus()
    handler = DecisionServiceHandlers(bus)
    handler.register()

    state = DecisionServiceState(handler=handler)

    def _capture_proposal(env: EventEnvelope) -> None:
        state.proposals.append(env.payload)

    bus.subscribe(topics.DECISION_PROPOSAL_CREATED_V1, _capture_proposal)

    lifespan = redis_stream_poll_lifespan(bus, [topics.FEATURES_ROW_GENERATED_V1])
    app = build_scaffold_app("decision_service", lifespan=lifespan)

    @app.post("/ingest/features-row")
    def ingest_features_row(payload: dict) -> dict[str, int]:
        env = EventEnvelope(
            event_type="features.row.generated",
            trace_id=str(payload.get("trace_id", "manual")),
            producer_service="feature_service",
            symbol=payload.get("symbol"),
            payload=payload,
        )
        bus.publish(topics.FEATURES_ROW_GENERATED_V1, env)
        if isinstance(bus, RedisStreamsMessageBus):
            bus.poll_once(topics.FEATURES_ROW_GENERATED_V1)
        return {"proposals": len(state.proposals)}

    @app.get("/events/recent")
    def events_recent() -> dict[str, list[dict]]:
        return {"proposals": list(state.proposals)}

    @app.get("/messaging")
    def messaging_info() -> dict[str, str]:
        backend = "redis_streams" if isinstance(bus, RedisStreamsMessageBus) else "in_memory"
        return {"messaging_backend": backend}

    return app
