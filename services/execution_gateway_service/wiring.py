"""Dependency wiring for execution gateway service."""

from __future__ import annotations

from dataclasses import dataclass, field

from fastapi import FastAPI

from services.common import build_scaffold_app
from services.execution_gateway_service.handlers import ExecutionGatewayHandlers
from shared.messaging import topics
from shared.messaging.envelope import EventEnvelope
from shared.messaging.in_memory import InMemoryMessageBus


@dataclass
class ExecutionGatewayState:
    handler: ExecutionGatewayHandlers
    order_acks: list[dict] = field(default_factory=list)
    order_fills: list[dict] = field(default_factory=list)
    order_rejections: list[dict] = field(default_factory=list)
    position_snapshots: list[dict] = field(default_factory=list)


def create_app() -> FastAPI:
    app = build_scaffold_app("execution_gateway_service")
    bus = InMemoryMessageBus()
    handler = ExecutionGatewayHandlers(bus)
    handler.register()

    state = ExecutionGatewayState(handler=handler)

    def _capture_ack(env: EventEnvelope) -> None:
        state.order_acks.append(env.payload)

    def _capture_fill(env: EventEnvelope) -> None:
        state.order_fills.append(env.payload)

    def _capture_rejected(env: EventEnvelope) -> None:
        state.order_rejections.append(env.payload)

    def _capture_position(env: EventEnvelope) -> None:
        state.position_snapshots.append(env.payload)

    bus.subscribe(topics.EXECUTION_ORDER_ACK_V1, _capture_ack)
    bus.subscribe(topics.EXECUTION_ORDER_FILL_V1, _capture_fill)
    bus.subscribe(topics.EXECUTION_ORDER_REJECTED_V1, _capture_rejected)
    bus.subscribe(topics.EXECUTION_POSITION_SNAPSHOT_V1, _capture_position)

    @app.post("/ingest/risk-accepted")
    def ingest_risk_accepted(payload: dict) -> dict[str, int]:
        env = EventEnvelope(
            event_type="risk.intent.accepted",
            trace_id=str(payload.get("trace_id", "manual")),
            producer_service="risk_service",
            symbol=payload.get("symbol"),
            payload=payload,
        )
        bus.publish(topics.RISK_INTENT_ACCEPTED_V1, env)
        return {
            "submitted_orders": len(state.handler.submitted_orders),
            "acks": len(state.order_acks),
            "fills": len(state.order_fills),
            "positions": len(state.position_snapshots),
            "rejections": len(state.order_rejections),
        }

    @app.get("/events/recent")
    def events_recent() -> dict[str, list[dict]]:
        return {
            "submitted_orders": list(state.handler.submitted_orders),
            "acks": list(state.order_acks),
            "fills": list(state.order_fills),
            "positions": list(state.position_snapshots),
            "rejections": list(state.order_rejections),
        }

    return app
