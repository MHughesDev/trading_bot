from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from datetime import UTC, datetime
from typing import Any

from app.contracts.decisions import ExecutionReport, OrderIntent


class AdapterError(RuntimeError):
    """Raised when an execution adapter cannot complete a request."""


@dataclass(slots=True)
class PositionSnapshot:
    symbol: str
    quantity: float
    avg_entry_price: float | None = None
    unrealized_pnl: float | None = None
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass(slots=True)
class AccountSnapshot:
    equity: float | None = None
    buying_power: float | None = None
    cash: float | None = None
    updated_at: datetime = field(default_factory=lambda: datetime.now(UTC))
    raw: dict[str, Any] = field(default_factory=dict)


class ExecutionAdapter(ABC):
    """
    Adapter abstraction for execution venues.

    Contract required by the V3 spec:
    - submit_order(order_intent)
    - cancel_order(id)
    - fetch_positions()
    """

    name: str = "base"

    @abstractmethod
    async def submit_order(self, order_intent: OrderIntent) -> ExecutionReport:
        raise NotImplementedError

    @abstractmethod
    async def cancel_order(self, order_id: str) -> dict[str, Any]:
        raise NotImplementedError

    @abstractmethod
    async def fetch_positions(self) -> list[PositionSnapshot]:
        raise NotImplementedError

    async def fetch_account(self) -> AccountSnapshot:
        """
        Optional account fetch capability.

        Default implementation returns an empty snapshot; concrete adapters can override.
        """
        return AccountSnapshot()
