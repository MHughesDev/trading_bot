"""In-process app scheduler (FB-AP-035)."""

from __future__ import annotations

import time

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


def test_status_includes_app_scheduler(client_no_auth: TestClient) -> None:
    r = client_no_auth.get("/status")
    assert r.status_code == 200
    assert "app_scheduler" in r.json()
    assert "enabled" in r.json()["app_scheduler"]


def test_nightly_tick_calls_job(monkeypatch: pytest.MonkeyPatch) -> None:
    calls: list[str] = []

    def fake_job(**_k: object) -> dict:
        calls.append("run")
        return {"ok": True}

    monkeypatch.setattr("orchestration.nightly_retrain.run_nightly_training_job", fake_job)
    cfg = AppSettings(scheduler_nightly_enabled=True, scheduler_nightly_interval_seconds=1)
    sch.start_app_background_scheduler(cfg)
    time.sleep(2.5)
    sch.stop_app_background_scheduler()
    assert len(calls) >= 1
