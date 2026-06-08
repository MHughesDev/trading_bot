"""ASGI entrypoint for control_plane_service."""

from __future__ import annotations

from services.control_plane_service.wiring import create_app

app = create_app()
