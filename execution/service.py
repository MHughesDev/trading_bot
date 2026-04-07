"""Single entry: risk-approved OrderIntent → venue adapter (spec execution router)."""

from __future__ import annotations

from app.config.settings import AppSettings
from app.contracts.orders import OrderIntent
from execution.adapters.base_adapter import ExecutionAdapter, OrderAck
from execution.intent_gate import require_execution_allowed
from execution.router import get_execution_adapter


class ExecutionService:
    """Use this instead of calling adapters directly so signing + routing stay centralized."""

    def __init__(self, settings: AppSettings, adapter: ExecutionAdapter | None = None) -> None:
        self._settings = settings
        self._adapter = adapter or get_execution_adapter(settings)

    @property
    def adapter(self) -> ExecutionAdapter:
        return self._adapter

    async def submit_order(self, intent: OrderIntent) -> OrderAck:
        require_execution_allowed(intent, self._settings)
        return await self._adapter.submit_order(intent)
