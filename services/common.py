"""Common helpers for service scaffolding apps."""

from __future__ import annotations

from datetime import datetime, timezone

from fastapi import FastAPI


def build_scaffold_app(service_name: str) -> FastAPI:
    """Create a minimal service app with health/readiness/status endpoints."""
    app = FastAPI(title=f"NautilusMonster {service_name}", version="0.1.0")

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
