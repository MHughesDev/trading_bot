"""Redis Streams bus adapter baseline.

Implements publish + handler registration + explicit polling (`poll_once`) for
incremental rollout before introducing a full background consumer loop.
"""

from __future__ import annotations

import json
from collections import defaultdict
from typing import Any

from pydantic import ValidationError

from shared.messaging.bus import MessageBus, MessageHandler
from shared.messaging.envelope import EventEnvelope


class RedisStreamsMessageBus(MessageBus):
    """Redis Streams adapter with explicit polling API."""

    def __init__(self, redis_url: str, client: Any | None = None) -> None:
        self._redis_url = redis_url
        self._client = client or self._build_client(redis_url)
        self._handlers: dict[str, list[MessageHandler]] = defaultdict(list)
        self._offsets: dict[str, str] = defaultdict(lambda: "0-0")

    @staticmethod
    def _build_client(redis_url: str) -> Any:
        try:
            import redis
        except Exception as exc:  # noqa: BLE001
            raise RuntimeError(
                "redis package is required for RedisStreamsMessageBus"
            ) from exc
        return redis.Redis.from_url(redis_url, decode_responses=True)

    def publish(self, topic: str, envelope: EventEnvelope) -> None:
        self._client.xadd(topic, {"envelope": envelope.model_dump_json()})

    def subscribe(self, topic: str, handler: MessageHandler) -> None:
        self._handlers[topic].append(handler)

    def poll_once(self, topic: str, *, count: int = 20) -> int:
        """Read and dispatch at most ``count`` stream events for one topic."""
        start = self._offsets[topic]
        rows = self._client.xrange(topic, min=start, count=count)
        consumed = 0
        for event_id, fields in rows:
            if event_id == start:
                continue
            raw = fields.get("envelope") if isinstance(fields, dict) else None
            if not raw:
                self._offsets[topic] = event_id
                continue
            try:
                data = json.loads(raw)
                envelope = EventEnvelope.model_validate(data)
            except (json.JSONDecodeError, ValidationError):
                self._offsets[topic] = event_id
                continue
            for handler in self._handlers.get(topic, []):
                handler(envelope)
            consumed += 1
            self._offsets[topic] = event_id
        return consumed
