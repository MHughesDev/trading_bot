"""ASGI entrypoint for risk service."""

from __future__ import annotations

from services.risk_service.wiring import create_app

app = create_app()
