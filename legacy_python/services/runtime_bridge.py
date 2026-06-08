"""Bridge helpers to route runtime feature rows through microservice handoff."""

from __future__ import annotations

from typing import Any, Literal

from services.pipeline_handoff import wire_phase3_handoff
from shared.messaging import topics
from shared.messaging.bus import MessageBus
from shared.messaging.envelope import EventEnvelope
from shared.messaging.runner import poll_topic_once
from shared.messaging.trace import new_trace_id

ExecutionGatewayMode = Literal["in_process", "external"]


class RuntimeHandoffBridge:
    """Publishes feature events and drives downstream handoff consumers."""

    def __init__(
        self,
        bus: MessageBus,
        *,
        execution_gateway_mode: ExecutionGatewayMode = "in_process",
    ) -> None:
        self._bus = bus
        self._execution_gateway_mode = execution_gateway_mode
        register_exec = execution_gateway_mode == "in_process"
        self._execution = wire_phase3_handoff(bus, register_execution=register_exec)

    @property
    def submitted_orders(self) -> list[dict]:
        if self._execution is None:
            return []
        return self._execution.submitted_orders

    def process_feature_row(self, payload: dict[str, Any]) -> None:
        symbol = str(payload.get("symbol", ""))
        env = EventEnvelope(
            event_type="features.row.generated",
            trace_id=new_trace_id(),
            producer_service="runtime_bridge",
            symbol=symbol or None,
            payload=payload,
        )
        self._bus.publish(topics.FEATURES_ROW_GENERATED_V1, env)

        # For explicit-poll buses (Redis baseline), advance each stage once.
        poll_topic_once(self._bus, topics.FEATURES_ROW_GENERATED_V1)
        poll_topic_once(self._bus, topics.DECISION_PROPOSAL_CREATED_V1)
        if self._execution_gateway_mode == "in_process":
            poll_topic_once(self._bus, topics.RISK_INTENT_ACCEPTED_V1)
