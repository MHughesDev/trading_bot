from __future__ import annotations

import random
from dataclasses import dataclass
from typing import Any
from uuid import uuid4

from app.contracts.decisions import ExecutionReport, OrderIntent
from execution.adapters.base_adapter import AccountSnapshot, ExecutionAdapter, PositionSnapshot


@dataclass(slots=True)
class AlpacaPaperExecutionAdapter(ExecutionAdapter):
    """
    Alpaca paper execution adapter.

    This adapter intentionally represents paper execution with simulated immediate
    fills so paper and live can share the exact decision and risk path.
    """

    name: str = "alpaca_paper"
    _last_prices: dict[str, float] | None = None

    def __post_init__(self) -> None:
        if self._last_prices is None:
            self._last_prices = {}

    def update_last_price(self, symbol: str, price: float) -> None:
        if self._last_prices is None:
            self._last_prices = {}
        self._last_prices[symbol] = price

    async def submit_order(self, order_intent: OrderIntent) -> ExecutionReport:
        order_id = f"alp_{uuid4().hex[:16]}"
        slippage_bps = random.uniform(0.0, 4.0)
        last_price = self._last_prices.get(order_intent.symbol, 0.0) if self._last_prices else 0.0
        base_price = order_intent.limit_price or order_intent.stop_price or last_price
        if base_price <= 0:
            base_price = 1.0
        fill_price = base_price * (1 + slippage_bps / 10_000)

        return ExecutionReport(
            order_id=order_id,
            client_order_id=order_intent.decision_id,
            symbol=order_intent.symbol,
            side=order_intent.side,
            quantity=order_intent.quantity,
            filled_quantity=order_intent.quantity,
            avg_fill_price=fill_price,
            status="filled",
            adapter=self.name,
            raw={"simulated": True, "slippage_bps": slippage_bps},
        )

    async def cancel_order(self, order_id: str) -> dict[str, Any]:
        return {"order_id": order_id, "status": "cancelled", "adapter": self.name}

    async def fetch_positions(self) -> list[PositionSnapshot]:
        return []

    async def fetch_account(self) -> AccountSnapshot:
        return AccountSnapshot(equity=100_000.0, buying_power=100_000.0, cash=100_000.0)
