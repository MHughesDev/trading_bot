"""ASGI entrypoint for observability_writer_service."""

from __future__ import annotations

from services.observability_writer_service.wiring import create_app

app = create_app()
