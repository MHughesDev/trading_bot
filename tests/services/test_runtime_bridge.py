from __future__ import annotations

from services.runtime_bridge import RuntimeHandoffBridge
from shared.messaging.in_memory import InMemoryMessageBus
from shared.messaging.redis_streams import RedisStreamsMessageBus


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


def test_runtime_bridge_in_memory_handoff() -> None:
    bridge = RuntimeHandoffBridge(InMemoryMessageBus())
    bridge.process_feature_row(
        {"symbol": "BTC/USD", "direction": 1, "size_fraction": 0.1, "route_id": "SCALPING"}
    )
    assert bridge.submitted_orders
    assert bridge.submitted_orders[-1]["side"] == "buy"


def test_runtime_bridge_redis_poll_handoff() -> None:
    bus = RedisStreamsMessageBus("redis://unused", client=_FakeRedis())
    bridge = RuntimeHandoffBridge(bus)
    bridge.process_feature_row(
        {"symbol": "ETH/USD", "direction": 1, "size_fraction": 0.2, "route_id": "SCALPING"}
    )
    assert bridge.submitted_orders
    assert bridge.submitted_orders[-1]["symbol"] == "ETH/USD"


def test_runtime_bridge_external_mode_does_not_consume_risk_stream() -> None:
    """External execution gateway: accepted intents stay on stream for another process."""
    bus = RedisStreamsMessageBus("redis://unused", client=_FakeRedis())
    bridge = RuntimeHandoffBridge(bus, execution_gateway_mode="external")
    bridge.process_feature_row(
        {"symbol": "BTC/USD", "direction": 1, "size_fraction": 0.1, "route_id": "SCALPING"}
    )
    assert bridge.submitted_orders == []
