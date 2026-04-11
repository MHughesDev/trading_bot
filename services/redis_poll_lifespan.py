"""FastAPI lifespan: background poll loop for RedisStreamsMessageBus."""

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from shared.messaging.bus import MessageBus


@asynccontextmanager
async def redis_stream_poll_lifespan(
    bus: MessageBus,
    topics_to_poll: list[str],
    *,
    interval_s: float = 0.05,
) -> AsyncIterator[None]:
    """Run ``poll_once`` for each topic in a loop when bus is Redis Streams."""
    from shared.messaging.redis_streams import RedisStreamsMessageBus

    if not isinstance(bus, RedisStreamsMessageBus):
        yield
        return

    stop = asyncio.Event()

    async def _loop() -> None:
        while not stop.is_set():
            for t in topics_to_poll:
                bus.poll_once(t)
            await asyncio.sleep(interval_s)

    task = asyncio.create_task(_loop())
    try:
        yield
    finally:
        stop.set()
        await task
