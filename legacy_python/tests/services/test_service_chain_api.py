from __future__ import annotations

import pytest

pytest.importorskip("fastapi")
pytest.importorskip("starlette.testclient")

from fastapi.testclient import TestClient

from services.decision_service.main import app as decision_app
from services.risk_service.main import app as risk_app
from services.execution_gateway_service.main import app as execution_app


def test_decision_to_risk_to_execution_api_chain() -> None:
    decision = TestClient(decision_app)
    risk = TestClient(risk_app)
    execution = TestClient(execution_app)

    d = decision.post(
        "/ingest/features-row",
        json={"symbol": "BTC/USD", "direction": 1, "size_fraction": 0.3, "route_id": "SCALPING"},
    )
    assert d.status_code == 200
    proposal = decision.get("/events/recent").json()["proposals"][-1]

    r = risk.post("/ingest/decision-proposal", json=proposal)
    assert r.status_code == 200
    accepted = risk.get("/events/recent").json()["accepted"][-1]

    e = execution.post("/ingest/risk-accepted", json=accepted)
    assert e.status_code == 200
    assert e.json()["submitted_orders"] >= 1
