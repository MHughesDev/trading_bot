from __future__ import annotations

from shared.messaging.envelope import EventEnvelope
from shared.messaging.in_memory import InMemoryMessageBus
from shared.messaging.redis_streams import RedisStreamsMessageBus
from shared.messaging.trace import new_trace_id


def test_in_memory_bus_publish_subscribe() -> None:
    bus = InMemoryMessageBus()
    received: list[EventEnvelope] = []

    bus.subscribe("decision.proposal.created.v1", received.append)
    env = EventEnvelope(
        event_type="decision.proposal.created",
        trace_id=new_trace_id(),
        producer_service="decision_service",
        payload={"symbol": "BTC/USD"},
    )

    bus.publish("decision.proposal.created.v1", env)

    assert len(received) == 1
    assert received[0].payload["symbol"] == "BTC/USD"


def test_redis_bus_requires_dependency_or_client() -> None:
    try:
        bus = RedisStreamsMessageBus("redis://localhost:6379/0")
    except RuntimeError as exc:
        assert "redis package is required" in str(exc)
        return

    assert isinstance(bus, RedisStreamsMessageBus)


def test_trace_id_helper_generates_values() -> None:
    t1 = new_trace_id()
    t2 = new_trace_id()
    assert t1
    assert t2
    assert t1 != t2


def test_in_memory_bus_dead_letters_after_retries() -> None:
    bus = InMemoryMessageBus(max_retries=1)

    def _boom(_env: EventEnvelope) -> None:
        raise RuntimeError("boom")

    bus.subscribe("risk.intent.accepted.v1", _boom)
    env = EventEnvelope(
        event_type="risk.intent.accepted",
        trace_id=new_trace_id(),
        producer_service="risk_service",
        payload={"symbol": "BTC/USD"},
    )

    bus.publish("risk.intent.accepted.v1", env)

    dlq = bus.dead_letter("risk.intent.accepted.v1")
    assert len(dlq) == 1
    assert dlq[0].payload["symbol"] == "BTC/USD"
