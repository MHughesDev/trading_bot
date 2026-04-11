"""Risk service topic handlers for Phase 3 handoff wiring."""

from __future__ import annotations

import os
from uuid import uuid4

from shared.messaging import topics
from shared.messaging.bus import MessageBus
from shared.messaging.envelope import EventEnvelope
from shared.messaging.security import sign_payload


class RiskServiceHandlers:
    """Consumes proposals and emits accepted/blocked intents."""

    def __init__(self, bus: MessageBus) -> None:
        self._bus = bus
        self._signing_secret = os.getenv("NM_RISK_SIGNING_SECRET", "dev-risk-secret")

    def on_proposal(self, env: EventEnvelope) -> None:
        direction = int(env.payload.get("direction", 0))
        symbol = str(env.payload.get("symbol", env.symbol or ""))
        if direction == 0:
            blocked = EventEnvelope(
                event_type="risk.intent.blocked",
                event_version="v1",
                trace_id=env.trace_id,
                correlation_id=str(env.event_id),
                producer_service="risk_service",
                symbol=symbol or None,
                partition_key=symbol or None,
                payload={
                    "symbol": symbol,
                    "blocked_reason": "zero_direction",
                    "proposal": env.payload,
                },
            )
            self._bus.publish(topics.RISK_INTENT_BLOCKED_V1, blocked)
            return

        signed_intent = {
            "intent_id": str(uuid4()),
            "symbol": symbol,
            "side": "buy" if direction > 0 else "sell",
            "quantity": float(env.payload.get("size_fraction", 0.1)),
            "metadata": {"route_id": env.payload.get("route_id", "SCALPING")},
        }
        signature = sign_payload(signed_intent, self._signing_secret)
        accepted = EventEnvelope(
            event_type="risk.intent.accepted",
            event_version="v1",
            trace_id=env.trace_id,
            correlation_id=str(env.event_id),
            producer_service="risk_service",
            symbol=symbol or None,
            partition_key=symbol or None,
            payload={"signed_intent": signed_intent, "risk_signature": signature},
        )
        self._bus.publish(topics.RISK_INTENT_ACCEPTED_V1, accepted)

    def register(self) -> None:
        self._bus.subscribe(topics.DECISION_PROPOSAL_CREATED_V1, self.on_proposal)
