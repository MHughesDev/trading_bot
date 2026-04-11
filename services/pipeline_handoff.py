"""Phase 3 local handoff wiring for decision -> risk -> execution topics."""

from __future__ import annotations

from services.decision_service.handlers import DecisionServiceHandlers
from services.execution_gateway_service.handlers import ExecutionGatewayHandlers
from services.risk_service.handlers import RiskServiceHandlers
from shared.messaging.bus import MessageBus


def wire_phase3_handoff(bus: MessageBus) -> ExecutionGatewayHandlers:
    """Register topic handlers for local event-driven handoff flow."""
    decision = DecisionServiceHandlers(bus)
    risk = RiskServiceHandlers(bus)
    execution = ExecutionGatewayHandlers(bus)
    decision.register()
    risk.register()
    execution.register()
    return execution
