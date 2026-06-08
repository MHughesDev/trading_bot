"""ASGI entrypoint for execution gateway service."""

from __future__ import annotations

from services.execution_gateway_service.wiring import create_app

app = create_app()
