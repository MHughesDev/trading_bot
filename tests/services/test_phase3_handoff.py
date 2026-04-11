from __future__ import annotations

from services.pipeline_handoff import wire_phase3_handoff
from services.execution_gateway_service.handlers import ExecutionGatewayHandlers
from shared.messaging import topics
from shared.messaging.envelope import EventEnvelope
from shared.messaging.in_memory import InMemoryMessageBus
from shared.messaging.trace import new_trace_id


def test_wire_phase3_without_execution_registers_decision_risk_only() -> None:
    bus = InMemoryMessageBus()
    accepted: list[EventEnvelope] = []
    bus.subscribe(topics.RISK_INTENT_ACCEPTED_V1, accepted.append)
    execution = wire_phase3_handoff(bus, register_execution=False)
    assert execution is None
    feature = EventEnvelope(
        event_type="features.row.generated",
        trace_id=new_trace_id(),
        producer_service="feature_service",
        symbol="BTC/USD",
        payload={"symbol": "BTC/USD", "direction": 1, "size_fraction": 0.1, "route_id": "SCALPING"},
    )
    bus.publish(topics.FEATURES_ROW_GENERATED_V1, feature)
    assert len(accepted) == 1
    assert accepted[0].payload.get("signed_intent")


def test_feature_to_execution_ack_handoff() -> None:
    bus = InMemoryMessageBus()
    execution = wire_phase3_handoff(bus)
    assert isinstance(execution, ExecutionGatewayHandlers)

    acks: list[EventEnvelope] = []
    bus.subscribe(topics.EXECUTION_ORDER_ACK_V1, acks.append)

    feature = EventEnvelope(
        event_type="features.row.generated",
        trace_id=new_trace_id(),
        producer_service="feature_service",
        symbol="BTC/USD",
        payload={
            "symbol": "BTC/USD",
            "direction": 1,
            "size_fraction": 0.25,
            "route_id": "SCALPING",
        },
    )

    bus.publish(topics.FEATURES_ROW_GENERATED_V1, feature)

    assert len(execution.submitted_orders) == 1
    assert execution.submitted_orders[0]["side"] == "buy"
    assert len(acks) == 1
    assert acks[0].payload["status"] == "accepted"


def test_zero_direction_blocks_before_execution() -> None:
    bus = InMemoryMessageBus()
    execution = wire_phase3_handoff(bus)

    blocked: list[EventEnvelope] = []
    bus.subscribe(topics.RISK_INTENT_BLOCKED_V1, blocked.append)

    feature = EventEnvelope(
        event_type="features.row.generated",
        trace_id=new_trace_id(),
        producer_service="feature_service",
        symbol="ETH/USD",
        payload={
            "symbol": "ETH/USD",
            "direction": 0,
            "size_fraction": 0.1,
            "route_id": "NO_TRADE",
        },
    )

    bus.publish(topics.FEATURES_ROW_GENERATED_V1, feature)

    assert len(blocked) == 1
    assert blocked[0].payload["blocked_reason"] == "zero_direction"
    assert execution.submitted_orders == []


def test_invalid_signature_is_rejected() -> None:
    bus = InMemoryMessageBus()
    execution = wire_phase3_handoff(bus)

    rejected: list[EventEnvelope] = []
    bus.subscribe(topics.EXECUTION_ORDER_REJECTED_V1, rejected.append)

    bad = EventEnvelope(
        event_type="risk.intent.accepted",
        trace_id=new_trace_id(),
        producer_service="risk_service",
        symbol="BTC/USD",
        payload={
            "signed_intent": {
                "intent_id": "dup-1",
                "symbol": "BTC/USD",
                "side": "buy",
                "quantity": 1.0,
                "metadata": {"route_id": "SCALPING"},
            },
            "risk_signature": "bad",
        },
    )

    bus.publish(topics.RISK_INTENT_ACCEPTED_V1, bad)

    assert execution.submitted_orders == []
    assert len(rejected) == 1
    assert rejected[0].payload["reason"] == "invalid_signature"


def test_duplicate_intent_id_is_idempotent() -> None:
    bus = InMemoryMessageBus()
    execution = wire_phase3_handoff(bus)

    # normal feature flow generates one valid signed intent
    feature = EventEnvelope(
        event_type="features.row.generated",
        trace_id=new_trace_id(),
        producer_service="feature_service",
        symbol="BTC/USD",
        payload={"symbol": "BTC/USD", "direction": 1, "size_fraction": 0.2, "route_id": "SCALPING"},
    )
    bus.publish(topics.FEATURES_ROW_GENERATED_V1, feature)
    assert len(execution.submitted_orders) == 1

    # replay same accepted intent should be ignored (simulate duplicate)
    from shared.messaging.security import sign_payload

    signed = {
        "intent_id": "same-id",
        "symbol": "BTC/USD",
        "side": "buy",
        "quantity": 0.2,
        "metadata": {"route_id": "SCALPING"},
    }
    sig = sign_payload(signed, "dev-risk-secret")
    env = EventEnvelope(
        event_type="risk.intent.accepted",
        trace_id=new_trace_id(),
        producer_service="risk_service",
        symbol="BTC/USD",
        payload={"signed_intent": signed, "risk_signature": sig},
    )
    bus.publish(topics.RISK_INTENT_ACCEPTED_V1, env)
    bus.publish(topics.RISK_INTENT_ACCEPTED_V1, env)

    # first duplicate accepted once, second ignored
    assert len([o for o in execution.submitted_orders if o.get("intent_id") == "same-id"]) == 1
