"""ASGI entrypoint for market_data_service."""

from __future__ import annotations

from services.market_data_service.wiring import create_app

app = create_app()
