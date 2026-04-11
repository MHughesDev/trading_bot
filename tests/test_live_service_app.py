"""services/live_service_app ASGI (HTTP-only mode for tests)."""

from __future__ import annotations

import pytest

pytest.importorskip("fastapi")
from fastapi.testclient import TestClient

from services.live_service_app import create_app


def test_live_service_app_health_without_loop(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_LIVE_SERVICE_APP_START_LOOP", "false")
    client = TestClient(create_app())
    assert client.get("/healthz").json()["status"] == "ok"
    assert client.get("/readyz").json()["status"] == "ready"
    body = client.get("/status").json()
    assert body["service"] == "live_runtime"
