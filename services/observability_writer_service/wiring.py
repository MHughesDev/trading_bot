"""Observability writer: subscribe to execution + risk-blocked topics (Phase 7 scaffold)."""

from __future__ import annotations

import asyncio
import json
import logging
from collections import deque
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from dataclasses import dataclass, field
from datetime import UTC, datetime

from fastapi import FastAPI

from app.config.settings import load_settings
from data_plane.storage.questdb import QuestDBWriter
from services.common import build_scaffold_app
from services.redis_poll_lifespan import redis_stream_poll_lifespan
from shared.messaging import topics
from shared.messaging.bus import MessageBus
from shared.messaging.envelope import EventEnvelope
from shared.messaging.factory import create_message_bus
from shared.messaging.redis_streams import RedisStreamsMessageBus

# Bounded in-memory ring buffer per category (QuestDB wiring would replace this later).
_MAX_EVENTS = 500

logger = logging.getLogger(__name__)


@dataclass
class ObservabilityEventBuffers:
    """Ring buffers for async observability consumption (in-memory until QuestDB)."""

    acks: deque[dict] = field(default_factory=lambda: deque(maxlen=_MAX_EVENTS))
    fills: deque[dict] = field(default_factory=lambda: deque(maxlen=_MAX_EVENTS))
    rejections: deque[dict] = field(default_factory=lambda: deque(maxlen=_MAX_EVENTS))
    positions: deque[dict] = field(default_factory=lambda: deque(maxlen=_MAX_EVENTS))
    risk_blocked: deque[dict] = field(default_factory=lambda: deque(maxlen=_MAX_EVENTS))


def register_observability_writer_handlers(
    bus: MessageBus,
    buf: ObservabilityEventBuffers,
    *,
    questdb_pending: list[tuple[str, EventEnvelope]] | None = None,
) -> None:
    """Subscribe handlers; used by ASGI app and unit tests."""

    def _wrap(e: EventEnvelope) -> dict:
        return {"event_type": e.event_type, "payload": e.payload, "trace_id": e.trace_id}

    def _maybe_persist(topic: str, e: EventEnvelope) -> None:
        if questdb_pending is not None:
            questdb_pending.append((topic, e))

    def _ack(e: EventEnvelope) -> None:
        buf.acks.append(_wrap(e))
        _maybe_persist(topics.EXECUTION_ORDER_ACK_V1, e)

    def _fill(e: EventEnvelope) -> None:
        buf.fills.append(_wrap(e))
        _maybe_persist(topics.EXECUTION_ORDER_FILL_V1, e)

    def _rej(e: EventEnvelope) -> None:
        buf.rejections.append(_wrap(e))
        _maybe_persist(topics.EXECUTION_ORDER_REJECTED_V1, e)

    def _pos(e: EventEnvelope) -> None:
        buf.positions.append(_wrap(e))
        _maybe_persist(topics.EXECUTION_POSITION_SNAPSHOT_V1, e)

    def _blocked(e: EventEnvelope) -> None:
        buf.risk_blocked.append(_wrap(e))
        _maybe_persist(topics.RISK_INTENT_BLOCKED_V1, e)

    bus.subscribe(topics.EXECUTION_ORDER_ACK_V1, _ack)
    bus.subscribe(topics.EXECUTION_ORDER_FILL_V1, _fill)
    bus.subscribe(topics.EXECUTION_ORDER_REJECTED_V1, _rej)
    bus.subscribe(topics.EXECUTION_POSITION_SNAPSHOT_V1, _pos)
    bus.subscribe(topics.RISK_INTENT_BLOCKED_V1, _blocked)


def _rows_from_pending(
    pending: list[tuple[str, EventEnvelope]],
) -> list[tuple[datetime, str, str, str, str | None, str]]:
    rows: list[tuple[datetime, str, str, str, str | None, str]] = []
    for topic, env in pending:
        ts = env.ts_event if env.ts_event.tzinfo else env.ts_event.replace(tzinfo=UTC)
        sym = env.symbol
        payload = json.dumps(env.payload, default=str)
        rows.append((ts, topic, env.event_type, env.trace_id, sym, payload))
    pending.clear()
    return rows


@asynccontextmanager
async def _observability_lifespan(
    bus: MessageBus,
    poll_topics: list[str],
    questdb_pending: list[tuple[str, EventEnvelope]],
) -> AsyncIterator[None]:
    settings = load_settings()
    persist = settings.questdb_persist_microservice_events
    if not persist:
        async with redis_stream_poll_lifespan(bus, poll_topics):
            yield
        return

    writer = QuestDBWriter(
        settings.questdb_host,
        settings.questdb_port,
        settings.questdb_user,
        settings.questdb_password.get_secret_value(),
        settings.questdb_database,
        batch_max_rows=settings.questdb_batch_max_rows,
    )
    stop = asyncio.Event()

    async def _flush_loop() -> None:
        await writer.connect()
        try:
            while not stop.is_set():
                await asyncio.sleep(max(0.5, settings.questdb_flush_interval_seconds))
                if questdb_pending:
                    batch = _rows_from_pending(questdb_pending)
                    if batch:
                        try:
                            await writer.insert_microservice_events_batch(batch)
                        except Exception as e:  # noqa: BLE001
                            logger.warning("QuestDB microservice_events flush failed: %s", e)
        finally:
            try:
                if questdb_pending:
                    batch = _rows_from_pending(questdb_pending)
                    if batch:
                        await writer.insert_microservice_events_batch(batch)
            except Exception as e:  # noqa: BLE001
                logger.warning("QuestDB microservice_events final flush failed: %s", e)
            await writer.aclose()

    flush_task = asyncio.create_task(_flush_loop())
    try:
        async with redis_stream_poll_lifespan(bus, poll_topics):
            yield
    finally:
        stop.set()
        await flush_task


def create_app() -> FastAPI:
    bus = create_message_bus()
    buf = ObservabilityEventBuffers()
    settings = load_settings()
    questdb_pending: list[tuple[str, EventEnvelope]] | None = (
        [] if settings.questdb_persist_microservice_events else None
    )
    register_observability_writer_handlers(bus, buf, questdb_pending=questdb_pending)

    poll_topics = [
        topics.EXECUTION_ORDER_ACK_V1,
        topics.EXECUTION_ORDER_FILL_V1,
        topics.EXECUTION_ORDER_REJECTED_V1,
        topics.EXECUTION_POSITION_SNAPSHOT_V1,
        topics.RISK_INTENT_BLOCKED_V1,
    ]
    if settings.questdb_persist_microservice_events and questdb_pending is not None:
        lifespan = _observability_lifespan(bus, poll_topics, questdb_pending)
    else:
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
