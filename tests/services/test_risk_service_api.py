from __future__ import annotations

import pytest

pytest.importorskip("fastapi")
pytest.importorskip("starlette.testclient")

from fastapi.testclient import TestClient

from services.risk_service.main import app


def test_risk_service_accepts_nonzero_direction() -> None:
    client = TestClient(app)
    res = client.post(
        "/ingest/decision-proposal",
        json={"symbol": "BTC/USD", "direction": 1, "size_fraction": 0.2, "route_id": "SCALPING"},
    )
    assert res.status_code == 200
    body = res.json()
    assert body["accepted"] >= 1

    events = client.get("/events/recent")
    assert events.status_code == 200
    payload = events.json()
    assert payload["accepted"]
    accepted = payload["accepted"][-1]
    assert "order_intent" in accepted
    assert "message_signature" in accepted


def test_risk_service_blocks_zero_direction() -> None:
    client = TestClient(app)
    res = client.post(
        "/ingest/decision-proposal",
        json={"symbol": "ETH/USD", "direction": 0, "size_fraction": 0.1, "route_id": "NO_TRADE"},
    )
    assert res.status_code == 200
    assert res.json()["blocked"] >= 1

    events = client.get("/events/recent")
    blocked = events.json()["blocked"]
    assert blocked
    assert blocked[-1]["blocked_reason"] == "zero_direction"
