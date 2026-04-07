from __future__ import annotations

from dataclasses import dataclass

from app.contracts.common import ExecutionMode
from app.contracts.decisions import ExecutionReport, OrderIntent
from execution.adapters.alpaca_paper_adapter import AlpacaPaperExecutionAdapter
from execution.adapters.base_adapter import AccountSnapshot, ExecutionAdapter, PositionSnapshot
from execution.adapters.coinbase_adapter import CoinbaseExecutionAdapter


@dataclass(slots=True)
class ExecutionRouter:
    mode: ExecutionMode
    coinbase_adapter: CoinbaseExecutionAdapter
    alpaca_paper_adapter: AlpacaPaperExecutionAdapter

    def _active_adapter(self) -> ExecutionAdapter:
        # Spec requirement:
        # if mode == "paper": use Alpaca
        # else: use Coinbase
        if self.mode == ExecutionMode.PAPER:
            return self.alpaca_paper_adapter
        return self.coinbase_adapter

    def set_mode(self, mode: ExecutionMode) -> None:
        self.mode = mode

    async def submit_order(self, order: OrderIntent) -> ExecutionReport:
        return await self._active_adapter().submit_order(order)

    async def cancel_order(self, order_id: str) -> dict[str, str]:
        raw = await self._active_adapter().cancel_order(order_id)
        return {k: str(v) for k, v in raw.items()}

    async def fetch_positions(self) -> list[PositionSnapshot]:
        return await self._active_adapter().fetch_positions()

    async def fetch_account(self) -> AccountSnapshot:
        return await self._active_adapter().fetch_account()

    def active_adapter_name(self) -> str:
        return self._active_adapter().name
