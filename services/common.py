"""Common helpers for service scaffolding apps."""

from __future__ import annotations

from collections.abc import Callable
from contextlib import AbstractAsyncContextManager
from datetime import datetime, timezone
from typing import Any

from fastapi import FastAPI


def build_scaffold_app(
    service_name: str,
    *,
    lifespan: Callable[[FastAPI], AbstractAsyncContextManager[Any]] | None = None,
) -> FastAPI:
    """Create a minimal service app with health/readiness/status endpoints."""
    kwargs: dict[str, Any] = {"title": f"Trading Bot {service_name}", "version": "0.1.0"}
    if lifespan is not None:
        kwargs["lifespan"] = lifespan
    app = FastAPI(**kwargs)

    @app.get("/healthz")
    def healthz() -> dict[str, str]:
        return {"status": "ok", "service": service_name}

    @app.get("/readyz")
    def readyz() -> dict[str, str]:
        return {"status": "ready", "service": service_name}

    @app.get("/status")
    def status() -> dict[str, str]:
        return {
            "service": service_name,
            "phase": "microservice_scaffold",
            "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        }

    return app
