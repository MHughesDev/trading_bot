"""Observability writer service wiring."""

from __future__ import annotations

from services.observability_writer_service.wiring import (
    ObservabilityEventBuffers,
    register_observability_writer_handlers,
)
from services.pipeline_handoff import wire_phase3_handoff
from shared.messaging import topics
from shared.messaging.envelope import EventEnvelope
from shared.messaging.in_memory import InMemoryMessageBus
from shared.messaging.trace import new_trace_id


def test_observability_writer_captures_execution_events_from_handoff() -> None:
    bus = InMemoryMessageBus()
    buf = ObservabilityEventBuffers()
    register_observability_writer_handlers(bus, buf)
    wire_phase3_handoff(bus)

    feature = EventEnvelope(
        event_type="features.row.generated",
        trace_id=new_trace_id(),
        producer_service="test",
        symbol="BTC-USD",
        payload={
            "symbol": "BTC-USD",
            "direction": 1,
            "size_fraction": 0.1,
            "route_id": "SCALPING",
        },
    )
    bus.publish(topics.FEATURES_ROW_GENERATED_V1, feature)

    assert buf.acks
    assert buf.fills
    assert buf.positions
    assert not buf.risk_blocked


def test_observability_writer_captures_risk_blocked() -> None:
    bus = InMemoryMessageBus()
    buf = ObservabilityEventBuffers()
    register_observability_writer_handlers(bus, buf)
    wire_phase3_handoff(bus)

    feature = EventEnvelope(
        event_type="features.row.generated",
        trace_id=new_trace_id(),
        producer_service="test",
        symbol="ETH-USD",
        payload={
            "symbol": "ETH-USD",
            "direction": 0,
            "size_fraction": 0.1,
            "route_id": "NO_TRADE",
        },
    )
    bus.publish(topics.FEATURES_ROW_GENERATED_V1, feature)

    assert buf.risk_blocked
    assert "zero_direction" in str(buf.risk_blocked[-1])
