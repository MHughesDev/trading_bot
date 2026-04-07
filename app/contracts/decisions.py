from __future__ import annotations

from decimal import Decimal
from enum import StrEnum

from pydantic import BaseModel, Field


class RouteId(StrEnum):
    NO_TRADE = "NO_TRADE"
    SCALPING = "SCALPING"
    INTRADAY = "INTRADAY"
    SWING = "SWING"


class RouteDecision(BaseModel):
    route_id: RouteId
    confidence: float = Field(ge=0.0, le=1.0)
    ranking: list[RouteId] = Field(default_factory=list)


class ActionProposal(BaseModel):
    """Per-route action before risk approval."""

    symbol: str
    route_id: RouteId
    direction: int  # +1 long, -1 short, 0 flat
    size_fraction: float = Field(ge=0.0, le=1.0, description="Fraction of max slot")
    stop_distance_pct: float = Field(ge=0.0)
    order_type: str = "market"
    expiry_seconds: int | None = None


class TradeAction(BaseModel):
    """Risk-approved action mapped to execution."""

    symbol: str
    side: str  # buy | sell
    quantity: Decimal
    order_type: str = "market"
    limit_price: Decimal | None = None
    stop_price: Decimal | None = None
    time_in_force: str = "gtc"
    route_id: RouteId
