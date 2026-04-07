from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

from pydantic import BaseModel, ConfigDict, Field

from app.contracts.common import OrderType, RouteId, Side, TimeInForce


class RouteDecision(BaseModel):
    model_config = ConfigDict(extra="forbid")

    route_id: RouteId
    confidence: float = Field(ge=0.0, le=1.0)
    ranking: list[RouteId] = Field(default_factory=list)
    reasons: list[str] = Field(default_factory=list)


class ActionIntent(BaseModel):
    model_config = ConfigDict(extra="forbid")

    symbol: str
    side: Side
    quantity: float = Field(gt=0)
    order_type: OrderType = OrderType.MARKET
    stop_distance: float | None = Field(default=None, gt=0)
    expiry_seconds: int | None = Field(default=None, gt=0)
    metadata: dict[str, Any] = Field(default_factory=dict)


class OrderIntent(BaseModel):
    model_config = ConfigDict(extra="forbid")

    symbol: str
    side: Side
    quantity: float = Field(gt=0)
    order_type: OrderType
    limit_price: float | None = Field(default=None, gt=0)
    stop_price: float | None = Field(default=None, gt=0)
    time_in_force: TimeInForce = TimeInForce.GTC
    route_id: RouteId
    decision_id: str
    created_at: datetime = Field(default_factory=lambda: datetime.now(UTC))
    metadata: dict[str, Any] = Field(default_factory=dict)


class RiskDecision(BaseModel):
    model_config = ConfigDict(extra="forbid")

    approved: bool
    reason: str
    adjusted_quantity: float | None = Field(default=None, ge=0)
    adjusted_order_type: OrderType | None = None
    blocked_by: list[str] = Field(default_factory=list)


class ExecutionReport(BaseModel):
    model_config = ConfigDict(extra="forbid")

    order_id: str
    client_order_id: str | None = None
    symbol: str
    side: Side
    quantity: float
    filled_quantity: float = 0.0
    avg_fill_price: float | None = None
    status: str
    adapter: str
    raw: dict[str, Any] = Field(default_factory=dict)
