"""Single entry: risk-approved OrderIntent → venue adapter (spec execution router)."""

from __future__ import annotations

from app.config.settings import AppSettings
from app.contracts.orders import OrderIntent
from app.runtime.asset_execution_mode import effective_execution_mode
from execution.adapters.base_adapter import ExecutionAdapter, OrderAck
from execution.intent_gate import require_execution_allowed
from execution.router import create_execution_adapter


class ExecutionService:
    """Use this instead of calling adapters directly so signing + routing stay centralized."""

    def __init__(self, settings: AppSettings, adapter: ExecutionAdapter | None = None) -> None:
        self._settings = settings
        self._fixed_adapter = adapter
        self._adapter = adapter or create_execution_adapter(settings)

    @property
    def adapter(self) -> ExecutionAdapter:
        return self._adapter

    def adapter_for_symbol(self, symbol: str) -> ExecutionAdapter:
        """Venue adapter for ``symbol`` (per-asset paper/live); same routing as ``submit_order``."""
        if self._fixed_adapter is not None:
            return self._fixed_adapter
        mode = effective_execution_mode(symbol, self._settings)
        if mode == self._settings.execution_mode:
            return self._adapter
        return create_execution_adapter(self._settings.model_copy(update={"execution_mode": mode}))

    def _adapter_for_intent(self, intent: OrderIntent) -> ExecutionAdapter:
        """Route by per-symbol execution mode unless a fixed adapter was injected (tests/stub)."""
        return self.adapter_for_symbol(intent.symbol)

    async def submit_order(self, intent: OrderIntent) -> OrderAck:
        require_execution_allowed(intent, self._settings)
        adapter = self._adapter_for_intent(intent)
        return await adapter.submit_order(intent)
