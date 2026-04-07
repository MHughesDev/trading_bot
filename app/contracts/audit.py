from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

from pydantic import BaseModel, ConfigDict, Field

from app.contracts.decisions import ActionIntent, OrderIntent, RiskDecision, RouteDecision


class DecisionTrace(BaseModel):
    """Auditable decision trace from signal generation to execution intent."""

    model_config = ConfigDict(extra="forbid")

    trace_id: str
    timestamp: datetime = Field(default_factory=lambda: datetime.now(UTC))
    symbol: str
    features: dict[str, float] = Field(default_factory=dict)
    memory_features: dict[str, float] = Field(default_factory=dict)
    forecast: dict[str, Any] = Field(default_factory=dict)
    regime: dict[str, Any] = Field(default_factory=dict)
    route_decision: RouteDecision
    action_intent: ActionIntent | None = None
    risk_decision: RiskDecision | None = None
    order_intent: OrderIntent | None = None
    notes: list[str] = Field(default_factory=list)
