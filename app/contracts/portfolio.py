"""Normalized portfolio position rows for control plane / UI (FB-DASH-03)."""

from __future__ import annotations

from decimal import Decimal

from pydantic import BaseModel, Field

from execution.adapters.base_adapter import PositionSnapshot


class PortfolioPositionRow(BaseModel):
    """Venue-agnostic open position for API and dashboard."""

    symbol: str
    quantity: str = Field(description="Signed quantity as decimal string")
    avg_entry_price: str | None = None
    unrealized_pnl: str | None = None
    venue_adapter: str = Field(description="Adapter name, e.g. alpaca_paper, coinbase_live, stub")


def position_snapshot_to_row(
    snap: PositionSnapshot,
    *,
    venue_adapter: str,
) -> PortfolioPositionRow:
    """Map adapter ``PositionSnapshot`` to a stable JSON-friendly row."""
    return PortfolioPositionRow(
        symbol=snap.symbol,
        quantity=str(snap.quantity),
        avg_entry_price=None if snap.avg_entry_price is None else str(snap.avg_entry_price),
        unrealized_pnl=None if snap.unrealized_pnl is None else str(snap.unrealized_pnl),
        venue_adapter=venue_adapter,
    )


def decimal_from_str(s: str | None) -> Decimal | None:
    if s is None or s == "":
        return None
    return Decimal(s)
