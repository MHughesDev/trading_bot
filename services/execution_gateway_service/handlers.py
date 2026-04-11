"""Execution gateway topic handlers for Phase 3 handoff wiring."""

from __future__ import annotations

import os
from collections import defaultdict

from shared.messaging import topics
from shared.messaging.bus import MessageBus
from shared.messaging.envelope import EventEnvelope
from shared.messaging.security import verify_payload


class ExecutionGatewayHandlers:
    """Consumes accepted risk intents and emits execution events."""

    def __init__(self, bus: MessageBus) -> None:
        self._bus = bus
        self._signing_secret = os.getenv("NM_RISK_SIGNING_SECRET", "dev-risk-secret")
        self.submitted_orders: list[dict] = []
        self._seen_intent_ids: set[str] = set()
        self._position_by_symbol: dict[str, float] = defaultdict(float)

    def _emit_rejected(self, env: EventEnvelope, reason: str) -> None:
        rej = EventEnvelope(
            event_type="execution.order.rejected",
            event_version="v1",
            trace_id=env.trace_id,
            correlation_id=str(env.event_id),
            producer_service="execution_gateway_service",
            symbol=env.symbol,
            partition_key=env.partition_key,
            payload={"reason": reason, "raw": env.payload},
        )
        self._bus.publish(topics.EXECUTION_ORDER_REJECTED_V1, rej)

    def on_accepted_intent(self, env: EventEnvelope) -> None:
        signed_intent = env.payload.get("signed_intent")
        signature = env.payload.get("risk_signature")
        if not isinstance(signed_intent, dict) or not isinstance(signature, str):
            self._emit_rejected(env, "missing_signature_payload")
            return
        if not verify_payload(signed_intent, signature, self._signing_secret):
            self._emit_rejected(env, "invalid_signature")
            return

        intent_id = str(signed_intent.get("intent_id", ""))
        if not intent_id:
            self._emit_rejected(env, "missing_intent_id")
            return
        if intent_id in self._seen_intent_ids:
            return
        self._seen_intent_ids.add(intent_id)

        self.submitted_orders.append(signed_intent)
        symbol = str(signed_intent.get("symbol", env.symbol or ""))
        qty = float(signed_intent.get("quantity", 0.0))
        side = str(signed_intent.get("side", "buy"))
        self._position_by_symbol[symbol] += qty if side == "buy" else -qty

        ack = EventEnvelope(
            event_type="execution.order.ack",
            event_version="v1",
            trace_id=env.trace_id,
            correlation_id=intent_id,
            producer_service="execution_gateway_service",
            symbol=symbol or None,
            partition_key=env.partition_key,
            payload={"symbol": symbol, "status": "accepted", "order_ref": f"ack-{intent_id}"},
        )
        fill = EventEnvelope(
            event_type="execution.order.fill",
            event_version="v1",
            trace_id=env.trace_id,
            correlation_id=intent_id,
            producer_service="execution_gateway_service",
            symbol=symbol or None,
            partition_key=env.partition_key,
            payload={"symbol": symbol, "filled_qty": qty, "side": side},
        )
        pos = EventEnvelope(
            event_type="execution.position.snapshot",
            event_version="v1",
            trace_id=env.trace_id,
            correlation_id=intent_id,
            producer_service="execution_gateway_service",
            symbol=symbol or None,
            partition_key=env.partition_key,
            payload={"symbol": symbol, "position_qty": self._position_by_symbol[symbol]},
        )
        self._bus.publish(topics.EXECUTION_ORDER_ACK_V1, ack)
        self._bus.publish(topics.EXECUTION_ORDER_FILL_V1, fill)
        self._bus.publish(topics.EXECUTION_POSITION_SNAPSHOT_V1, pos)

    def register(self) -> None:
        self._bus.subscribe(topics.RISK_INTENT_ACCEPTED_V1, self.on_accepted_intent)
