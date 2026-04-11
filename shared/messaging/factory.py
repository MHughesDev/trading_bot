"""Factory helpers for selecting a message bus backend."""

from __future__ import annotations

import os

from shared.messaging.bus import MessageBus
from shared.messaging.in_memory import InMemoryMessageBus
from shared.messaging.redis_streams import RedisStreamsMessageBus


def create_message_bus() -> MessageBus:
    """Create the configured message bus backend.

    Env:
    - NM_MESSAGING_BACKEND: `in_memory` (default) | `redis_streams`
    - NM_REDIS_URL: Redis URL used when backend is `redis_streams`
    """
    backend = os.getenv("NM_MESSAGING_BACKEND", "in_memory").strip().lower()
    if backend == "redis_streams":
        redis_url = os.getenv("NM_REDIS_URL", "redis://localhost:6379/0")
        return RedisStreamsMessageBus(redis_url)
    return InMemoryMessageBus()
