from decimal import Decimal
from enum import Enum
from typing import Any

from pydantic import BaseModel, Field, field_validator


class OrderSide(str, Enum):
    BUY = "buy"
    SELL = "sell"


class OrderType(str, Enum):
    MARKET = "market"
    LIMIT = "limit"
    STOP = "stop"
    STOP_LIMIT = "stop_limit"


class TimeInForce(str, Enum):
    GTC = "gtc"
    IOC = "ioc"
    FOK = "fok"
    GTD = "gtd"


_FORBIDDEN_META = frozenset(
    {"headline", "raw_text", "news_text", "article", "body", "tweet"}
)


class OrderIntent(BaseModel):
    """Risk-approved intent passed to an execution adapter (not a raw exchange order)."""

    symbol: str
    side: OrderSide
    quantity: Decimal
    order_type: OrderType = OrderType.MARKET
    limit_price: Decimal | None = None
    stop_price: Decimal | None = None
    time_in_force: TimeInForce = TimeInForce.GTC
    client_order_id: str | None = None
    metadata: dict[str, Any] = Field(default_factory=dict)

    @field_validator("metadata")
    @classmethod
    def no_raw_text_in_metadata(cls, v: dict[str, Any]) -> dict[str, Any]:
        lower = {k.lower() for k in v}
        if lower & _FORBIDDEN_META:
            raise ValueError("OrderIntent metadata must not carry raw news/text fields")
        return v
