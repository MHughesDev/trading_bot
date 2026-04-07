"""Lightweight asyncio periodic tasks (Prefect can orchestrate heavier flows)."""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Awaitable, Callable

logger = logging.getLogger(__name__)


async def run_every(
    interval_seconds: float,
    coro_factory: Callable[[], Awaitable[None]],
    *,
    name: str = "task",
) -> None:
    """Run coro_factory() every interval_seconds until cancelled."""
    while True:
        try:
            await coro_factory()
        except asyncio.CancelledError:
            raise
        except Exception:
            logger.exception("scheduled task %s failed", name)
        await asyncio.sleep(interval_seconds)
