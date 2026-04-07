"""Simple portfolio tracker for backtests."""

from __future__ import annotations

from decimal import Decimal


class PortfolioTracker:
    def __init__(self, cash: Decimal = Decimal("100000")) -> None:
        self.cash = cash
        self.positions: dict[str, Decimal] = {}

    def market_value(self, prices: dict[str, float]) -> Decimal:
        mv = self.cash
        for sym, qty in self.positions.items():
            p = prices.get(sym, 0.0)
            mv += qty * Decimal(str(p))
        return mv
