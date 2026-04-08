"""Portfolio tracker for backtests — cash, positions, optional equity curve."""

from __future__ import annotations

from decimal import Decimal


class PortfolioTracker:
    def __init__(self, cash: Decimal = Decimal("100000")) -> None:
        self.cash = cash
        self.positions: dict[str, Decimal] = {}

    def apply_trade(
        self,
        symbol: str,
        qty: Decimal,
        side: str,
        cash_delta: Decimal,
    ) -> None:
        """Update cash and signed position after a simulated fill."""
        self.cash += cash_delta
        if side == "buy":
            self.positions[symbol] = self.positions.get(symbol, Decimal(0)) + qty
        else:
            self.positions[symbol] = self.positions.get(symbol, Decimal(0)) - qty

    def market_value(self, prices: dict[str, float]) -> Decimal:
        mv = self.cash
        for sym, qty in self.positions.items():
            p = prices.get(sym, 0.0)
            mv += qty * Decimal(str(p))
        return mv
