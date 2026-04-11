"""Portfolio DTO mapping."""

from __future__ import annotations

from decimal import Decimal

from app.contracts.portfolio import PortfolioPositionRow, position_snapshot_to_row
from execution.adapters.base_adapter import PositionSnapshot


def test_position_snapshot_to_row():
    snap = PositionSnapshot(
        symbol="BTC-USD",
        quantity=Decimal("1.5"),
        avg_entry_price=Decimal("90000"),
        unrealized_pnl=Decimal("12.34"),
    )
    row = position_snapshot_to_row(snap, venue_adapter="alpaca_paper")
    assert isinstance(row, PortfolioPositionRow)
    d = row.model_dump(mode="json")
    assert d["symbol"] == "BTC-USD"
    assert d["quantity"] == "1.5"
    assert d["avg_entry_price"] == "90000"
    assert d["unrealized_pnl"] == "12.34"
    assert d["venue_adapter"] == "alpaca_paper"
