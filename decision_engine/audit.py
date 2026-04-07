"""Structured decision traces for audit (JSON-serializable)."""

from __future__ import annotations

from typing import Any

from app.contracts.decisions import ActionProposal, RouteDecision
from app.contracts.forecast import ForecastOutput
from app.contracts.orders import OrderIntent
from app.contracts.regime import RegimeOutput
from app.contracts.risk import RiskState


def decision_trace(
    *,
    symbol: str,
    regime: RegimeOutput,
    forecast: ForecastOutput,
    route: RouteDecision,
    proposal: ActionProposal | None,
    risk: RiskState,
    trade_allowed: bool,
    order_intent: OrderIntent | None = None,
    block_reason: str | None = None,
    correlation_id: str | None = None,
) -> dict[str, Any]:
    """Single blob suitable for QuestDB decision_traces or structured logs."""
    out: dict[str, Any] = {
        "symbol": symbol,
        "correlation_id": correlation_id,
        "regime": regime.model_dump(mode="json"),
        "forecast": forecast.model_dump(mode="json"),
        "route": route.model_dump(mode="json"),
        "proposal": proposal.model_dump(mode="json") if proposal else None,
        "risk": risk.model_dump(mode="json"),
        "trade_allowed": trade_allowed,
        "block_reason": block_reason,
        "order_intent": order_intent.model_dump(mode="json") if order_intent else None,
    }
    return out
