"""Normalized portfolio position rows for control plane / UI (FB-DASH-03)."""

from __future__ import annotations

from decimal import Decimal

from pydantic import BaseModel, Field

from app.config.settings import AppSettings
from execution.adapters.base_adapter import PositionSnapshot
from execution.mark_price import compute_unrealized_pnl, effective_mark_price_source


class PortfolioPositionRow(BaseModel):
    """Venue-agnostic open position for API and dashboard."""

    symbol: str
    quantity: str = Field(description="Signed quantity as decimal string")
    avg_entry_price: str | None = None
    unrealized_pnl: str | None = None
    mark_price: str | None = Field(
        default=None,
        description="Mark used for uPnL when computed (e.g. Kraken mid) or from venue when available",
    )
    mark_price_source: str | None = Field(
        default=None,
        description="kraken_mid | venue | none — see portfolio_mark_price_source_* in settings",
    )
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
        mark_price=None,
        mark_price_source=None,
        venue_adapter=venue_adapter,
    )


def decimal_from_str(s: str | None) -> Decimal | None:
    if s is None or s == "":
        return None
    return Decimal(s)


def enrich_row_with_mark(
    row: PortfolioPositionRow,
    *,
    settings: AppSettings,
    kraken_mid_by_symbol: dict[str, Decimal],
) -> PortfolioPositionRow:
    """
    Set ``mark_price``, ``mark_price_source``, and ``unrealized_pnl`` per
    ``portfolio_mark_price_source_*`` and available prices (FB-DASH-04-02).
    """
    src = effective_mark_price_source(settings)
    qty = decimal_from_str(row.quantity)
    avg = decimal_from_str(row.avg_entry_price)
    venue_u = decimal_from_str(row.unrealized_pnl)

    if src == "venue_only":
        mp = None
        if venue_u is not None and qty is not None and qty != 0 and avg is not None:
            mp = venue_u / qty + avg
        return row.model_copy(
            update={
                "mark_price": None if mp is None else str(mp),
                "mark_price_source": "venue" if mp is not None else None,
                "unrealized_pnl": None if venue_u is None else str(venue_u),
            }
        )

    # kraken_mid
    mark = kraken_mid_by_symbol.get(row.symbol)
    if mark is not None and qty is not None and avg is not None:
        u = compute_unrealized_pnl(qty, avg, mark)
        return row.model_copy(
            update={
                "mark_price": str(mark),
                "mark_price_source": "kraken_mid",
                "unrealized_pnl": None if u is None else str(u),
            }
        )
    if venue_u is not None:
        mp = None
        if qty is not None and qty != 0 and avg is not None:
            mp = venue_u / qty + avg
        return row.model_copy(
            update={
                "mark_price": None if mp is None else str(mp),
                "mark_price_source": "venue" if mp is not None else None,
                "unrealized_pnl": str(venue_u),
            }
        )
    return row.model_copy(update={"mark_price_source": None})
