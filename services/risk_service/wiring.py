"""Dependency wiring for risk service."""

from __future__ import annotations

from dataclasses import dataclass, field

from fastapi import FastAPI

from app.config.settings import load_settings

from services.common import build_scaffold_app
from services.redis_poll_lifespan import redis_stream_poll_lifespan
from services.risk_service.handlers import RiskServiceHandlers
from shared.messaging import topics
from shared.messaging.envelope import EventEnvelope
from shared.messaging.factory import create_message_bus
from shared.messaging.redis_streams import RedisStreamsMessageBus


@dataclass
class RiskServiceState:
    handler: RiskServiceHandlers
    accepted: list[dict] = field(default_factory=list)
    blocked: list[dict] = field(default_factory=list)


def create_app() -> FastAPI:
    bus = create_message_bus()
    handler = RiskServiceHandlers(bus, settings=load_settings())
    handler.register()

    state = RiskServiceState(handler=handler)

    def _capture_accepted(env: EventEnvelope) -> None:
        state.accepted.append(env.payload)

    def _capture_blocked(env: EventEnvelope) -> None:
        state.blocked.append(env.payload)

    bus.subscribe(topics.RISK_INTENT_ACCEPTED_V1, _capture_accepted)
    bus.subscribe(topics.RISK_INTENT_BLOCKED_V1, _capture_blocked)

    lifespan = redis_stream_poll_lifespan(bus, [topics.DECISION_PROPOSAL_CREATED_V1])
    app = build_scaffold_app("risk_service", lifespan=lifespan)

    @app.post("/ingest/decision-proposal")
    def ingest_decision_proposal(payload: dict) -> dict[str, int]:
        env = EventEnvelope(
            event_type="decision.proposal.created",
            trace_id=str(payload.get("trace_id", "manual")),
            producer_service="decision_service",
            symbol=payload.get("symbol"),
            payload=payload,
        )
        bus.publish(topics.DECISION_PROPOSAL_CREATED_V1, env)
        if isinstance(bus, RedisStreamsMessageBus):
            bus.poll_once(topics.DECISION_PROPOSAL_CREATED_V1)
        return {"accepted": len(state.accepted), "blocked": len(state.blocked)}

    @app.get("/events/recent")
    def events_recent() -> dict[str, list[dict]]:
        return {"accepted": list(state.accepted), "blocked": list(state.blocked)}

    @app.get("/messaging")
    def messaging_info() -> dict[str, str]:
        backend = "redis_streams" if isinstance(bus, RedisStreamsMessageBus) else "in_memory"
        return {"messaging_backend": backend}

    return app
