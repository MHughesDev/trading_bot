"""Control plane optional microservice health aggregation."""

from __future__ import annotations

import pytest

pytest.importorskip("fastapi")
pytest.importorskip("httpx")
from fastapi.testclient import TestClient

from control_plane.api import app


def test_microservices_health_endpoint_shape() -> None:
    client = TestClient(app)
    r = client.get("/microservices/health")
    assert r.status_code == 200
    body = r.json()
    assert "services" in body
    assert "host" in body
    assert isinstance(body["services"], dict)
