"""Observability writer: subscribe to execution + risk-blocked topics (Phase 7 scaffold)."""

from __future__ import annotations

from collections import deque
from dataclasses import dataclass, field

from fastapi import FastAPI

from services.common import build_scaffold_app
from services.redis_poll_lifespan import redis_stream_poll_lifespan
from shared.messaging import topics
from shared.messaging.bus import MessageBus
from shared.messaging.envelope import EventEnvelope
from shared.messaging.factory import create_message_bus
from shared.messaging.redis_streams import RedisStreamsMessageBus

# Bounded in-memory ring buffer per category (QuestDB wiring would replace this later).
_MAX_EVENTS = 500


@dataclass
class ObservabilityEventBuffers:
    """Ring buffers for async observability consumption (in-memory until QuestDB)."""

    acks: deque[dict] = field(default_factory=lambda: deque(maxlen=_MAX_EVENTS))
    fills: deque[dict] = field(default_factory=lambda: deque(maxlen=_MAX_EVENTS))
    rejections: deque[dict] = field(default_factory=lambda: deque(maxlen=_MAX_EVENTS))
    positions: deque[dict] = field(default_factory=lambda: deque(maxlen=_MAX_EVENTS))
    risk_blocked: deque[dict] = field(default_factory=lambda: deque(maxlen=_MAX_EVENTS))


def register_observability_writer_handlers(bus: MessageBus, buf: ObservabilityEventBuffers) -> None:
    """Subscribe handlers; used by ASGI app and unit tests."""

    def _wrap(e: EventEnvelope) -> dict:
        return {"event_type": e.event_type, "payload": e.payload, "trace_id": e.trace_id}

    def _ack(e: EventEnvelope) -> None:
        buf.acks.append(_wrap(e))

    def _fill(e: EventEnvelope) -> None:
        buf.fills.append(_wrap(e))

    def _rej(e: EventEnvelope) -> None:
        buf.rejections.append(_wrap(e))

    def _pos(e: EventEnvelope) -> None:
        buf.positions.append(_wrap(e))

    def _blocked(e: EventEnvelope) -> None:
        buf.risk_blocked.append(_wrap(e))

    bus.subscribe(topics.EXECUTION_ORDER_ACK_V1, _ack)
    bus.subscribe(topics.EXECUTION_ORDER_FILL_V1, _fill)
    bus.subscribe(topics.EXECUTION_ORDER_REJECTED_V1, _rej)
    bus.subscribe(topics.EXECUTION_POSITION_SNAPSHOT_V1, _pos)
    bus.subscribe(topics.RISK_INTENT_BLOCKED_V1, _blocked)


def create_app() -> FastAPI:
    bus = create_message_bus()
    buf = ObservabilityEventBuffers()
    register_observability_writer_handlers(bus, buf)

    poll_topics = [
        topics.EXECUTION_ORDER_ACK_V1,
        topics.EXECUTION_ORDER_FILL_V1,
        topics.EXECUTION_ORDER_REJECTED_V1,
        topics.EXECUTION_POSITION_SNAPSHOT_V1,
        topics.RISK_INTENT_BLOCKED_V1,
    ]
    lifespan = redis_stream_poll_lifespan(bus, poll_topics)
    app = build_scaffold_app("observability_writer_service", lifespan=lifespan)

    @app.get("/events/recent")
    def events_recent() -> dict[str, list[dict]]:
        return {
            "execution_acks": list(buf.acks),
            "execution_fills": list(buf.fills),
            "execution_rejections": list(buf.rejections),
            "execution_positions": list(buf.positions),
            "risk_blocked": list(buf.risk_blocked),
        }

    @app.get("/messaging")
    def messaging_info() -> dict[str, str]:
        backend = "redis_streams" if isinstance(bus, RedisStreamsMessageBus) else "in_memory"
        return {"messaging_backend": backend}

    return app
