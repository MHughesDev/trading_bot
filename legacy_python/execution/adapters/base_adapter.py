"""
Execution adapter contract: abstract exchange/broker behind OrderIntent.

Risk engine produces OrderIntent; adapters map to venue-specific APIs (Coinbase live, Alpaca paper).
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from decimal import Decimal
from typing import Any

from pydantic import BaseModel, Field

from app.contracts.orders import OrderIntent


class PositionSnapshot(BaseModel):
    """Venue-agnostic position for risk and reporting."""

    symbol: str
    quantity: Decimal
    avg_entry_price: Decimal | None = None
    unrealized_pnl: Decimal | None = None
    raw: dict[str, Any] = Field(default_factory=dict)


class OrderAck(BaseModel):
    """Acknowledgement after submit (venue-specific id + status)."""

    adapter: str
    order_id: str
    status: str
    raw: dict[str, Any] = Field(default_factory=dict)


class ExecutionAdapter(ABC):
    """Base class for CoinbaseExecutionAdapter and AlpacaPaperExecutionAdapter."""

    @property
    @abstractmethod
    def name(self) -> str:
        """Stable adapter id for logs and metrics."""

    @abstractmethod
    async def submit_order(self, order: OrderIntent) -> OrderAck:
        """Submit a risk-approved order intent to the venue."""

    @abstractmethod
    async def cancel_order(self, order_id: str) -> bool:
        """Request cancellation; return True if the cancel was accepted."""

    @abstractmethod
    async def fetch_positions(self) -> list[PositionSnapshot]:
        """Return open positions for reconciliation and risk checks."""
