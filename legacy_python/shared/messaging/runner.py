"""Helpers for driving explicit polling buses in local loops."""

from __future__ import annotations

from shared.messaging.redis_streams import RedisStreamsMessageBus
from shared.messaging.bus import MessageBus


def poll_topic_once(bus: MessageBus, topic: str) -> int:
    """Poll one topic if bus supports explicit polling; otherwise no-op."""
    if isinstance(bus, RedisStreamsMessageBus):
        return bus.poll_once(topic)
    return 0
