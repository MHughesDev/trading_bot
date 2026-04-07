from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from dataclasses import dataclass

JobCallback = Callable[[], Awaitable[None]]


@dataclass(slots=True)
class ScheduledJob:
    name: str
    interval_seconds: float
    callback: JobCallback


class AsyncScheduler:
    def __init__(self) -> None:
        self._jobs: list[ScheduledJob] = []
        self._tasks: list[asyncio.Task[None]] = []
        self._running = False

    async def _runner(self, job: ScheduledJob) -> None:
        while self._running:
            await job.callback()
            await asyncio.sleep(job.interval_seconds)

    def schedule(self, job: ScheduledJob) -> None:
        self._jobs.append(job)
        if self._running:
            self._tasks.append(asyncio.create_task(self._runner(job), name=f"job:{job.name}"))

    async def start(self) -> None:
        self._running = True
        for job in self._jobs:
            self._tasks.append(asyncio.create_task(self._runner(job), name=f"job:{job.name}"))

    async def stop(self) -> None:
        self._running = False
        for task in self._tasks:
            task.cancel()
        for task in self._tasks:
            try:
                await task
            except asyncio.CancelledError:
                pass
        self._tasks.clear()
