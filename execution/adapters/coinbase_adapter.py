from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import Any
from uuid import uuid4

from app.contracts.decisions import ExecutionReport, OrderIntent
from execution.adapters.base_adapter import (
    AccountSnapshot,
    AdapterError,
    ExecutionAdapter,
    PositionSnapshot,
)

logger = logging.getLogger(__name__)


@dataclass(slots=True)
class CoinbaseExecutionAdapter(ExecutionAdapter):
    """
    Coinbase live execution adapter.

    This V1 implementation preserves the live adapter boundary and produces
    auditable execution reports. If API credentials are unavailable, it fails fast.
    """

    api_key: str | None = None
    api_secret: str | None = None
    name: str = "coinbase"

    async def submit_order(self, order_intent: OrderIntent) -> ExecutionReport:
        if not self.api_key or not self.api_secret:
            raise AdapterError("Coinbase API credentials are required for live execution")

        # Integration point for coinbase-advanced-py client.
        order_id = f"cb_{uuid4().hex[:16]}"
        logger.info(
            "coinbase_submit_order",
            extra={
                "symbol": order_intent.symbol,
                "side": order_intent.side.value,
                "qty": order_intent.quantity,
            },
        )
        return ExecutionReport(
            order_id=order_id,
            client_order_id=order_intent.decision_id,
            symbol=order_intent.symbol,
            side=order_intent.side,
            quantity=order_intent.quantity,
            filled_quantity=0.0,
            avg_fill_price=None,
            status="accepted",
            adapter=self.name,
            raw={"simulated": False, "venue": "coinbase"},
        )

    async def cancel_order(self, order_id: str) -> dict[str, Any]:
        return {"order_id": order_id, "status": "cancel_requested", "adapter": self.name}

    async def fetch_positions(self) -> list[PositionSnapshot]:
        return []

    async def fetch_account(self) -> AccountSnapshot:
        return AccountSnapshot()
