"""GET /scheduler/nightly (FB-UX-012)."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from control_plane import api
from orchestration import app_scheduler as sch


@pytest.fixture(autouse=True)
def reset_scheduler():
    sch.reset_app_scheduler_for_tests()
    yield
    sch.reset_app_scheduler_for_tests()


@pytest.fixture
def client_no_auth(monkeypatch):
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    return TestClient(api.app)


def test_scheduler_nightly_endpoint(client_no_auth: TestClient) -> None:
    r = client_no_auth.get("/scheduler/nightly")
    assert r.status_code == 200
    j = r.json()
    assert "enabled" in j
    assert "interval_seconds" in j
    assert "last_tick_utc" in j
    assert "last_run_finished_utc" in j
    assert "next_run_after_utc" in j
    assert "last_error" in j
    assert "last_report" in j
    assert "nightly_per_asset_forecaster" in j
    assert "nightly_rl_requires_trade" in j


def test_status_app_scheduler_includes_next_fields(client_no_auth: TestClient) -> None:
    r = client_no_auth.get("/status")
    assert r.status_code == 200
    aps = r.json()["app_scheduler"]
    assert "last_run_finished_utc" in aps
    assert "next_run_after_utc" in aps
