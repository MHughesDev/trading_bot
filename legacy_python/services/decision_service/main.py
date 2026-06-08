"""ASGI entrypoint for decision service."""

from __future__ import annotations

from services.decision_service.wiring import create_app

app = create_app()
