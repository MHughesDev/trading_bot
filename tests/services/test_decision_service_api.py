from __future__ import annotations

import pytest

pytest.importorskip("fastapi")
pytest.importorskip("starlette.testclient")

from fastapi.testclient import TestClient

from services.decision_service.main import app


def test_decision_service_generates_proposal() -> None:
    client = TestClient(app)
    res = client.post(
        "/ingest/features-row",
        json={"symbol": "BTC/USD", "direction": 1, "size_fraction": 0.2, "route_id": "SCALPING"},
    )
    assert res.status_code == 200
    assert res.json()["proposals"] >= 1

    events = client.get("/events/recent")
    assert events.status_code == 200
    payload = events.json()
    assert payload["proposals"]
    assert payload["proposals"][-1]["symbol"] == "BTC/USD"
