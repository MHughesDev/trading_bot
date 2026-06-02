"""Request/response contracts for manual (human-operator or agent) orders.

Manual orders are the human-first trading surface: a caller specifies the exact
quantity/side and the platform still enforces the RiskEngine hard gates and risk-signs
the intent before it reaches a venue. The MCP server reuses the same contracts so an AI
agent and a human go through one audited path.
"""

from __future__ import annotations

from decimal import Decimal

from pydantic import BaseModel, Field


class ManualOrderRequest(BaseModel):
    symbol: str = Field(min_length=1, description="Canonical symbol, e.g. BTC-USD")
    side: str = Field(description="buy or sell")
    quantity: Decimal = Field(gt=0, description="Absolute quantity in base units")
    order_type: str = Field(default="market", description="market or limit")
    limit_price: Decimal | None = Field(default=None, gt=0, description="Required for limit orders")
    mid_price: float | None = Field(
        default=None, gt=0, description="Reference mark price used for the available-cash gate"
    )


class FlattenRequest(BaseModel):
    symbol: str = Field(min_length=1, description="Canonical symbol to flatten")


class ManualOrderResponse(BaseModel):
    submitted: bool
    symbol: str
    error: str | None = None
    blocked: list[str] = Field(default_factory=list)
    side: str | None = None
    quantity: str | None = None
    order_type: str | None = None
    position_qty_before: str | None = None
    acks: list[dict] = Field(default_factory=list)
