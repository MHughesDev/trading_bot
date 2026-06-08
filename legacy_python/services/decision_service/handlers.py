"""Decision service topic handlers for Phase 3 handoff wiring."""

from __future__ import annotations

from shared.messaging import topics
from shared.messaging.bus import MessageBus
from shared.messaging.envelope import EventEnvelope


class DecisionServiceHandlers:
    """Consumes features rows and emits decision proposals."""

    def __init__(self, bus: MessageBus) -> None:
        self._bus = bus

    def on_feature_row(self, env: EventEnvelope) -> None:
        symbol = str(env.payload.get("symbol", env.symbol or ""))
        proposal = EventEnvelope(
            event_type="decision.proposal.created",
            event_version="v1",
            trace_id=env.trace_id,
            correlation_id=str(env.event_id),
            producer_service="decision_service",
            symbol=symbol or None,
            partition_key=symbol or None,
            payload={
                "symbol": symbol,
                "direction": int(env.payload.get("direction", 1)),
                "size_fraction": float(env.payload.get("size_fraction", 0.1)),
                "route_id": str(env.payload.get("route_id", "SCALPING")),
                "order_type": "market",
            },
        )
        self._bus.publish(topics.DECISION_PROPOSAL_CREATED_V1, proposal)

    def register(self) -> None:
        self._bus.subscribe(topics.FEATURES_ROW_GENERATED_V1, self.on_feature_row)
