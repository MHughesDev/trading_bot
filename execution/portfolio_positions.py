"""Fetch open positions via the configured execution adapter (normalized for API)."""

from __future__ import annotations

from decimal import Decimal
from typing import Any

from app.config.settings import AppSettings
from app.contracts.portfolio import (
    PortfolioPositionRow,
    enrich_row_with_mark,
    position_snapshot_to_row,
)
from execution.mark_price import effective_mark_price_source, fetch_kraken_mid_prices
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
    mpsrc = effective_mark_price_source(settings)
    kraken_mid_by_symbol: dict[str, Decimal] = {}
    if mpsrc == "kraken_mid" and rows:
        try:
            kraken_mid_by_symbol = await fetch_kraken_mid_prices([r.symbol for r in rows])
        except Exception:
            kraken_mid_by_symbol = {}
    enriched = [
        enrich_row_with_mark(r, settings=settings, kraken_mid_by_symbol=kraken_mid_by_symbol)
        for r in rows
    ]
    return {
        "ok": True,
        "error": None,
        "positions": [r.model_dump(mode="json") for r in enriched],
        "adapter": adapter_name,
        "execution_mode": settings.execution_mode,
        "mark_price_policy": {
            "source": mpsrc,
            "execution_mode": settings.execution_mode,
        },
    }
