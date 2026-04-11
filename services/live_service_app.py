"""ASGI entry: HTTP probes + background Kraken live loop.

Same trading path as ``python -m app.runtime.live_service`` — runs
``run_live_loop`` in a background task while serving **/healthz** for orchestration.

Requires outbound network to Kraken when the loop runs. For compose/dev without
live keys, use scaffold services only; this process is optional.

Set ``NM_LIVE_SERVICE_APP_START_LOOP=false`` to serve HTTP only (tests / dry probes).
"""

from __future__ import annotations

import asyncio
import logging
import os
from contextlib import asynccontextmanager
from typing import AsyncIterator

from fastapi import FastAPI

from app.runtime.live_service import register_shutdown_signals, run_live_loop

logger = logging.getLogger(__name__)


def _start_loop_enabled() -> bool:
    return os.getenv("NM_LIVE_SERVICE_APP_START_LOOP", "true").strip().lower() in (
        "1",
        "true",
        "yes",
    )


@asynccontextmanager
async def lifespan(app: FastAPI) -> AsyncIterator[None]:
    if not _start_loop_enabled():
        yield
        return

    stop = asyncio.Event()
    register_shutdown_signals(stop)
    task = asyncio.create_task(run_live_loop(stop_event=stop))
    try:
        yield
    finally:
        stop.set()
        try:
            await asyncio.wait_for(task, timeout=45.0)
        except asyncio.TimeoutError:
            logger.warning("live loop task did not finish within 45s; cancelling")
            task.cancel()
            try:
                await task
            except asyncio.CancelledError:
                pass


def create_app() -> FastAPI:
    app = FastAPI(
        title="NautilusMonster live runtime",
        version="0.1.0",
        lifespan=lifespan,
    )

    @app.get("/healthz")
    def healthz() -> dict[str, str]:
        return {"status": "ok", "service": "live_runtime"}

    @app.get("/readyz")
    def readyz() -> dict[str, str]:
        return {"status": "ready", "service": "live_runtime"}

    @app.get("/status")
    def status() -> dict[str, str]:
        from datetime import datetime, timezone

        return {
            "service": "live_runtime",
            "phase": "kraken_live_loop",
            "note": "Wraps app.runtime.live_service.run_live_loop",
            "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        }

    return app


app = create_app()
