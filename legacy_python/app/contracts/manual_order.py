"""Request/response contracts for manual (human-operator or agent) orders.

Manual orders are the human-first trading surface: a caller specifies the exact
quantity/side and the platform still enforces the RiskEngine hard gates and risk-signs
the intent before it reaches a venue. The MCP server reuses the same contracts so an AI
agent and a human go through one audited path.
"""

from __future__ import annotations

from decimal import Decimal

from pydantic import BaseModel, Field, model_validator

_VALID_ORDER_TYPES = ("market", "limit", "stop", "stop_limit")
_VALID_TIF = ("gtc", "ioc", "fok", "gtd")


class ManualOrderRequest(BaseModel):
    symbol: str = Field(min_length=1, description="Canonical symbol, e.g. BTC-USD")
    side: str = Field(description="buy or sell")
    quantity: Decimal = Field(gt=0, description="Absolute quantity in base units")
    order_type: str = Field(default="market", description="market, limit, stop, or stop_limit")
    limit_price: Decimal | None = Field(
        default=None, gt=0, description="Required for limit and stop_limit orders"
    )
    stop_price: Decimal | None = Field(
        default=None, gt=0, description="Required for stop and stop_limit orders"
    )
    time_in_force: str = Field(default="gtc", description="gtc, ioc, fok, or gtd")
    mid_price: float | None = Field(
        default=None, gt=0, description="Reference mark price used for the available-cash gate"
    )

    @model_validator(mode="after")
    def _validate_type_and_prices(self) -> "ManualOrderRequest":
        ot = self.order_type
        if ot not in _VALID_ORDER_TYPES:
            raise ValueError(f"order_type must be one of {_VALID_ORDER_TYPES}")
        if self.time_in_force not in _VALID_TIF:
            raise ValueError(f"time_in_force must be one of {_VALID_TIF}")
        if ot in ("limit", "stop_limit") and self.limit_price is None:
            raise ValueError(f"{ot} order requires limit_price")
        if ot in ("stop", "stop_limit") and self.stop_price is None:
            raise ValueError(f"{ot} order requires stop_price")
        return self


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
