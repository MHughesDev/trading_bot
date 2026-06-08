# ruff: noqa: E402
from __future__ import annotations

import pytest

fastapi = pytest.importorskip("fastapi")
pytest.importorskip("starlette.testclient")
from fastapi.testclient import TestClient

from services.control_plane_service.main import app as control_plane_app
from services.decision_service.main import app as decision_app
from services.execution_gateway_service.main import app as execution_app
from services.feature_service.main import app as feature_app
from services.market_data_service.main import app as market_data_app
from services.observability_writer_service.main import app as observability_app
from services.risk_service.main import app as risk_app


@pytest.mark.parametrize(
    ("service_name", "app"),
    [
        ("market_data_service", market_data_app),
        ("feature_service", feature_app),
        ("decision_service", decision_app),
        ("risk_service", risk_app),
        ("execution_gateway_service", execution_app),
        ("control_plane_service", control_plane_app),
        ("observability_writer_service", observability_app),
    ],
)
def test_scaffold_endpoints(service_name: str, app) -> None:
    client = TestClient(app)

    health = client.get("/healthz")
    assert health.status_code == 200
    assert health.json() == {"status": "ok", "service": service_name}

    ready = client.get("/readyz")
    assert ready.status_code == 200
    assert ready.json() == {"status": "ready", "service": service_name}

    status = client.get("/status")
    assert status.status_code == 200
    body = status.json()
    assert body["service"] == service_name
    assert body["phase"] == "microservice_scaffold"
    assert "timestamp_utc" in body

    if service_name == "observability_writer_service":
        ev = client.get("/events/recent")
        assert ev.status_code == 200
        payload = ev.json()
        assert "execution_acks" in payload and "risk_blocked" in payload
        msg = client.get("/messaging")
        assert msg.status_code == 200
        assert "messaging_backend" in msg.json()
