from __future__ import annotations

import asyncio
import contextlib
from collections.abc import Awaitable, Callable


class EventLoopRuntime:
    """Simple runtime to coordinate long-running async services."""

    def __init__(self) -> None:
        self._tasks: list[asyncio.Task[None]] = []
        self._stopped = asyncio.Event()

    def add_service(self, name: str, coroutine_factory: Callable[[], Awaitable[None]]) -> None:
        task = asyncio.create_task(coroutine_factory(), name=f"svc:{name}")
        self._tasks.append(task)

    async def run_until_stopped(self) -> None:
        await self._stopped.wait()

    async def stop(self) -> None:
        self._stopped.set()
        for task in self._tasks:
            task.cancel()
        for task in self._tasks:
            with contextlib.suppress(asyncio.CancelledError):
                await task
