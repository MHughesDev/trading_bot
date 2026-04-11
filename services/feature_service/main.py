"""ASGI entrypoint for feature_service."""

from __future__ import annotations

from services.feature_service.wiring import create_app

app = create_app()
