from __future__ import annotations

import pytest

pytest.importorskip("fastapi")
pytest.importorskip("starlette.testclient")

from fastapi.testclient import TestClient

from services.execution_gateway_service.main import app
from shared.messaging.security import sign_payload


def test_execution_gateway_ingest_and_events() -> None:
    client = TestClient(app)

    signed_intent = {
        "intent_id": "intent-1",
        "symbol": "BTC/USD",
        "side": "buy",
        "quantity": 0.5,
        "metadata": {"route_id": "SCALPING"},
    }
    signature = sign_payload(signed_intent, "dev-risk-secret")

    res = client.post(
        "/ingest/risk-accepted",
        json={"symbol": "BTC/USD", "signed_intent": signed_intent, "risk_signature": signature},
    )
    assert res.status_code == 200
    body = res.json()
    assert body["submitted_orders"] == 1
    assert body["acks"] == 1
    assert body["fills"] == 1
    assert body["positions"] == 1
    assert body["rejections"] == 0

    events = client.get("/events/recent")
    assert events.status_code == 200
    payload = events.json()
    assert payload["submitted_orders"][0]["intent_id"] == "intent-1"


def test_execution_gateway_rejects_bad_signature() -> None:
    client = TestClient(app)
    signed_intent = {
        "intent_id": "intent-bad",
        "symbol": "ETH/USD",
        "side": "sell",
        "quantity": 1.0,
        "metadata": {"route_id": "SCALPING"},
    }

    res = client.post(
        "/ingest/risk-accepted",
        json={"symbol": "ETH/USD", "signed_intent": signed_intent, "risk_signature": "invalid"},
    )
    assert res.status_code == 200
    assert res.json()["rejections"] >= 1
