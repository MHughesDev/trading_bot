"""
Coinbase Advanced Trade live execution.

Signing: production orders require CDP JWT (ECDSA). V1 delegates to REST with keys when implemented.
"""

from __future__ import annotations

import logging
import uuid

from app.config.settings import AppSettings
from app.contracts.orders import OrderIntent
from execution.adapters.base_adapter import ExecutionAdapter, OrderAck, PositionSnapshot

logger = logging.getLogger(__name__)


class CoinbaseExecutionAdapter(ExecutionAdapter):
    def __init__(self, settings: AppSettings) -> None:
        self._settings = settings

    @property
    def name(self) -> str:
        return "coinbase"

    async def submit_order(self, order: OrderIntent) -> OrderAck:
        if not self._settings.coinbase_api_key or not self._settings.coinbase_api_secret:
            raise RuntimeError(
                "Coinbase API credentials missing (NM_COINBASE_API_KEY / NM_COINBASE_API_SECRET)"
            )
        logger.warning(
            "Coinbase signed order path not wired in V1 scaffold; returning synthetic ack for audit"
        )
        return OrderAck(
            adapter=self.name,
            order_id=str(uuid.uuid4()),
            status="pending_implementation",
            raw={"order": order.model_dump(mode="json")},
        )

    async def cancel_order(self, order_id: str) -> bool:
        logger.info("cancel_order %s (stub)", order_id)
        return False

    async def fetch_positions(self) -> list[PositionSnapshot]:
        return []
