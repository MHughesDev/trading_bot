"""Fetch open positions via the configured execution adapter (normalized for API)."""

from __future__ import annotations

from typing import Any

from app.config.settings import AppSettings
from app.contracts.portfolio import PortfolioPositionRow, position_snapshot_to_row
from execution.service import ExecutionService


async def fetch_portfolio_positions(settings: AppSettings) -> dict[str, Any]:
    """Return ``positions`` normalized rows; ``ok`` false if the venue call fails."""
    svc = ExecutionService(settings)
    adapter_name = svc.adapter.name
    try:
        snaps = await svc.adapter.fetch_positions()
    except Exception as e:
        return {
            "ok": False,
            "error": str(e),
            "positions": [],
            "adapter": adapter_name,
            "execution_mode": settings.execution_mode,
        }
    rows: list[PortfolioPositionRow] = [
        position_snapshot_to_row(s, venue_adapter=adapter_name) for s in snaps
    ]
    return {
        "ok": True,
        "error": None,
        "positions": [r.model_dump(mode="json") for r in rows],
        "adapter": adapter_name,
        "execution_mode": settings.execution_mode,
    }
