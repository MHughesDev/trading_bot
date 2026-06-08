"""Phase 3 local handoff wiring for decision -> risk -> execution topics."""

from __future__ import annotations

from services.decision_service.handlers import DecisionServiceHandlers
from services.execution_gateway_service.handlers import ExecutionGatewayHandlers
from services.risk_service.handlers import RiskServiceHandlers
from shared.messaging.bus import MessageBus


def wire_phase3_handoff(
    bus: MessageBus,
    *,
    register_execution: bool = True,
) -> ExecutionGatewayHandlers | None:
    """Register topic handlers for event-driven handoff flow.

    When ``register_execution`` is False, only decision + risk handlers are
    registered so ``risk.intent.accepted`` events stay on the bus/stream for
    an external ``execution_gateway_service`` process (Redis transport).
    """
    decision = DecisionServiceHandlers(bus)
    risk = RiskServiceHandlers(bus)
    decision.register()
    risk.register()
    execution: ExecutionGatewayHandlers | None = None
    if register_execution:
        execution = ExecutionGatewayHandlers(bus)
        execution.register()
    return execution
