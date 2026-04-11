"""Dependency wiring for execution gateway service."""

from __future__ import annotations

import asyncio
from contextlib import asynccontextmanager
from dataclasses import dataclass, field
from typing import AsyncIterator

from fastapi import FastAPI

from services.common import build_scaffold_app
from services.execution_gateway_bus import create_execution_gateway_bus
from services.execution_gateway_service.handlers import ExecutionGatewayHandlers
from shared.messaging import topics
from shared.messaging.envelope import EventEnvelope
from shared.messaging.redis_streams import RedisStreamsMessageBus


@dataclass
class ExecutionGatewayState:
    handler: ExecutionGatewayHandlers
    order_acks: list[dict] = field(default_factory=list)
    order_fills: list[dict] = field(default_factory=list)
    order_rejections: list[dict] = field(default_factory=list)
    position_snapshots: list[dict] = field(default_factory=list)


def create_app() -> FastAPI:
    bus = create_execution_gateway_bus()
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

    @asynccontextmanager
    async def lifespan(app: FastAPI) -> AsyncIterator[None]:
        app.state.bus = bus
        poll_task: asyncio.Task[None] | None = None
        poll_stop: asyncio.Event | None = None
        if isinstance(bus, RedisStreamsMessageBus):
            poll_stop = asyncio.Event()

            async def _poll_loop() -> None:
                assert poll_stop is not None
                while not poll_stop.is_set():
                    bus.poll_once(topics.RISK_INTENT_ACCEPTED_V1)
                    await asyncio.sleep(0.05)

            poll_task = asyncio.create_task(_poll_loop())
        try:
            yield
        finally:
            if poll_stop is not None:
                poll_stop.set()
            if poll_task is not None:
                await poll_task

    app = build_scaffold_app("execution_gateway_service", lifespan=lifespan)

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
        if isinstance(bus, RedisStreamsMessageBus):
            bus.poll_once(topics.RISK_INTENT_ACCEPTED_V1)
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

    @app.get("/messaging")
    def messaging_info() -> dict[str, str]:
        backend = "redis_streams" if isinstance(bus, RedisStreamsMessageBus) else "in_memory"
        return {"messaging_backend": backend}

    return app
