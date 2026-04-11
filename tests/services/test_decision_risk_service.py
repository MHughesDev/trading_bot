from __future__ import annotations

import importlib

import pytest

from shared.messaging.factory import create_message_bus
from shared.messaging.in_memory import InMemoryMessageBus
from shared.messaging.redis_streams import RedisStreamsMessageBus


def test_bus_factory_default_in_memory(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("NM_MESSAGING_BACKEND", raising=False)
    bus = create_message_bus()
    assert isinstance(bus, InMemoryMessageBus)


def test_bus_factory_explicit_in_memory(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_MESSAGING_BACKEND", "in_memory")
    bus = create_message_bus()
    assert isinstance(bus, InMemoryMessageBus)


def test_bus_factory_redis_streams(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_MESSAGING_BACKEND", "redis_streams")
    try:
        bus = create_message_bus()
        assert isinstance(bus, RedisStreamsMessageBus)
    except RuntimeError as exc:
        assert "redis package is required" in str(exc)


def test_decision_risk_simulate_endpoint() -> None:
    pytest.importorskip("fastapi")
    pytest.importorskip("starlette.testclient")
    from fastapi.testclient import TestClient

    mod = importlib.import_module("services.decision_risk_service.main")
    importlib.reload(mod)
    client = TestClient(mod.app)

    resp = client.post(
        "/simulate",
        json={"symbol": "BTC/USD", "direction": 1, "size_fraction": 0.25},
    )

    assert resp.status_code == 200
    body = resp.json()
    assert body["submitted_count"] >= 1
    assert body["submitted_orders"][-1]["side"] == "buy"
