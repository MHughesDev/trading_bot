"""In-process stub adapter for tests and dry-run (no external venue)."""

from __future__ import annotations

from app.config.settings import AppSettings
from app.contracts.orders import OrderIntent
from execution.adapters.base_adapter import ExecutionAdapter, OrderAck, PositionSnapshot


class StubExecutionAdapter(ExecutionAdapter):
    """Records submits; returns synthetic acks. No network."""

    def __init__(self, settings: AppSettings) -> None:
        self._settings = settings
        self.submitted: list[OrderIntent] = []

    @property
    def name(self) -> str:
        return "stub"

    async def submit_order(self, order: OrderIntent) -> OrderAck:
        self.submitted.append(order)
        cid = order.client_order_id or "stub"
        return OrderAck(adapter="stub", order_id=f"stub-{cid}", status="accepted", raw={})

    async def cancel_order(self, order_id: str) -> bool:
        return True

    async def fetch_positions(self) -> list[PositionSnapshot]:
        return []
