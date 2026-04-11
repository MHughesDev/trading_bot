"""Mock Alpaca paper adapter — no network; validates execution path in integration tests."""

from __future__ import annotations

from uuid import uuid4

from app.config.settings import AppSettings
from app.contracts.orders import OrderIntent
from execution.adapters.base_adapter import ExecutionAdapter, OrderAck, PositionSnapshot
from execution.alpaca_util import to_alpaca_crypto_symbol
from execution.intent_gate import require_execution_allowed


class MockAlpacaPaperExecutionAdapter(ExecutionAdapter):
    """Same contract as Alpaca paper: maps symbols, returns synthetic acks (no API calls)."""

    def __init__(self, settings: AppSettings) -> None:
        self._settings = settings

    @property
    def name(self) -> str:
        return "mock_alpaca_paper"

    async def submit_order(self, order: OrderIntent) -> OrderAck:
        require_execution_allowed(order, self._settings)
        sym = to_alpaca_crypto_symbol(order.symbol)
        oid = f"mock-{uuid4().hex[:12]}"
        return OrderAck(
            adapter=self.name,
            order_id=oid,
            status="filled",
            raw={"symbol": sym, "mock": True},
        )

    async def cancel_order(self, order_id: str) -> bool:
        return True

    async def fetch_positions(self) -> list[PositionSnapshot]:
        return []
