from __future__ import annotations

from shared.messaging.envelope import EventEnvelope
from shared.messaging.redis_streams import RedisStreamsMessageBus
from shared.messaging.trace import new_trace_id


class _FakeRedis:
    def __init__(self) -> None:
        self._streams: dict[str, list[tuple[str, dict[str, str]]]] = {}
        self._n = 0

    def xadd(self, topic: str, fields: dict[str, str]) -> str:
        self._n += 1
        event_id = f"{self._n}-0"
        self._streams.setdefault(topic, []).append((event_id, fields))
        return event_id

    def xrange(self, topic: str, min: str, count: int):
        items = self._streams.get(topic, [])
        out = []
        for event_id, fields in items:
            if event_id >= min:
                out.append((event_id, fields))
            if len(out) >= count:
                break
        return out


def test_redis_streams_publish_and_poll() -> None:
    fake = _FakeRedis()
    bus = RedisStreamsMessageBus("redis://unused", client=fake)

    got: list[EventEnvelope] = []
    bus.subscribe("decision.proposal.created.v1", got.append)

    env = EventEnvelope(
        event_type="decision.proposal.created",
        trace_id=new_trace_id(),
        producer_service="decision_service",
        payload={"symbol": "BTC/USD", "direction": 1},
    )
    bus.publish("decision.proposal.created.v1", env)

    n = bus.poll_once("decision.proposal.created.v1")
    assert n == 1
    assert len(got) == 1
    assert got[0].payload["symbol"] == "BTC/USD"


def test_redis_streams_poll_is_incremental() -> None:
    fake = _FakeRedis()
    bus = RedisStreamsMessageBus("redis://unused", client=fake)
    got: list[EventEnvelope] = []
    bus.subscribe("risk.intent.accepted.v1", got.append)

    env = EventEnvelope(
        event_type="risk.intent.accepted",
        trace_id=new_trace_id(),
        producer_service="risk_service",
        payload={"symbol": "ETH/USD"},
    )
    bus.publish("risk.intent.accepted.v1", env)

    first = bus.poll_once("risk.intent.accepted.v1")
    second = bus.poll_once("risk.intent.accepted.v1")

    assert first == 1
    assert second == 0
    assert len(got) == 1
