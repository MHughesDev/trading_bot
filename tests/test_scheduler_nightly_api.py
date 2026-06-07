"""GET /scheduler/nightly (FB-UX-012)."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from control_plane import api
from training_pipeline.orchestration import app_scheduler as sch


@pytest.fixture(autouse=True)
def reset_scheduler():
    sch.reset_app_scheduler_for_tests()
    yield
    sch.reset_app_scheduler_for_tests()


@pytest.fixture
def client_no_auth(monkeypatch):
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    return TestClient(api.app)


def test_scheduler_nightly_endpoint_is_decoupled(client_no_auth: TestClient) -> None:
    # Nightly training is decoupled (FB-AP-XXX): the endpoint is retained as an inert,
    # backward-compatible marker — no nightly job runs in the runtime.
    r = client_no_auth.get("/scheduler/nightly")
    assert r.status_code == 200
    j = r.json()
    assert j["enabled"] is False
    assert j["decoupled"] is True


def test_status_app_scheduler_reports_decoupled(client_no_auth: TestClient) -> None:
    r = client_no_auth.get("/status")
    assert r.status_code == 200
    aps = r.json()["app_scheduler"]
    assert aps["enabled"] is False
    assert aps["decoupled"] is True
