"""Structured decision traces for audit (JSON-serializable)."""

from __future__ import annotations

from typing import Any

from app.contracts.decisions import ActionProposal, RouteDecision
from app.contracts.forecast import ForecastOutput
from app.contracts.orders import OrderIntent
from app.contracts.regime import RegimeOutput
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.risk import RiskState


def _forecast_packet_summary(pkt: ForecastPacket | None) -> dict[str, object] | None:
    if pkt is None:
        return None
    return {
        "horizons": list(pkt.horizons),
        "q_med_head": [float(x) for x in pkt.q_med[:3]],
        "confidence_score": pkt.confidence_score,
        "ood_score": pkt.ood_score,
        "packet_schema_version": pkt.packet_schema_version,
        "source_checkpoint_id": pkt.source_checkpoint_id,
        "pipeline": pkt.forecast_diagnostics.get("pipeline"),
    }


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
    forecast_packet: ForecastPacket | None = None,
) -> dict[str, Any]:
    """Single blob suitable for QuestDB decision_traces or structured logs (no secrets)."""
    out: dict[str, Any] = {
        "symbol": symbol,
        "correlation_id": correlation_id,
        "regime": regime.model_dump(mode="json"),
        "forecast": forecast.model_dump(mode="json"),
        "forecast_packet_summary": _forecast_packet_summary(forecast_packet),
        "route": route.model_dump(mode="json"),
        "proposal": proposal.model_dump(mode="json") if proposal else None,
        "risk": risk.model_dump(mode="json"),
        "trade_allowed": trade_allowed,
        "block_reason": block_reason,
        "order_intent": order_intent.model_dump(mode="json") if order_intent else None,
    }
    return out
