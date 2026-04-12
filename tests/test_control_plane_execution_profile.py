"""Control plane /system/execution-profile endpoints."""

from __future__ import annotations

from pathlib import Path

from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from control_plane import api


def test_get_execution_profile_in_status(monkeypatch, tmp_path: Path):
    monkeypatch.setenv("NM_EXECUTION_PROFILE_LEGACY_API", "true")
    monkeypatch.setattr(api, "settings", AppSettings(execution_mode="paper"))
    from control_plane import execution_profile as ep

    monkeypatch.setattr(ep, "_STATE_PATH", tmp_path / "execution_profile.json")
    client = TestClient(api.app)
    r = client.get("/status")
    assert r.status_code == 200
    prof = r.json().get("execution_profile")
    assert prof is not None
    assert prof.get("legacy_api_enabled") is True
    assert prof["active_execution_mode"] == "paper"
    assert prof["restart_required"] is False


def test_status_execution_profile_without_legacy_api(monkeypatch):
    monkeypatch.delenv("NM_EXECUTION_PROFILE_LEGACY_API", raising=False)
    monkeypatch.setattr(api, "settings", AppSettings(execution_mode="paper"))
    client = TestClient(api.app)
    r = client.get("/status")
    prof = r.json().get("execution_profile")
    assert prof["legacy_api_enabled"] is False
    assert prof["default_execution_mode"] == "paper"


def test_post_execution_profile_sets_pending(monkeypatch, tmp_path: Path):
    monkeypatch.setenv("NM_EXECUTION_PROFILE_LEGACY_API", "true")
    monkeypatch.setattr(api, "settings", AppSettings(execution_mode="paper", control_plane_api_key=None))
    state_path = tmp_path / "execution_profile.json"
    monkeypatch.setattr("control_plane.execution_profile._STATE_PATH", state_path)
    dy = tmp_path / "default.yaml"
    dy.write_text("execution:\n  mode: paper\n", encoding="utf-8")
    envf = tmp_path / ".env"
    envf.write_text("NM_EXECUTION_MODE=paper\n", encoding="utf-8")
    monkeypatch.setattr("control_plane.execution_profile._DEFAULT_YAML", dy)
    monkeypatch.setattr("control_plane.execution_profile._REPO_ROOT", tmp_path)

    client = TestClient(api.app)
    r = client.post(
        "/system/execution-profile",
        json={"execution_mode": "live", "apply_to_config_files": True},
    )
    assert r.status_code == 200
    body = r.json()
    assert body["pending_execution_mode"] == "live"
    assert body["restart_required"] is True
    assert body["config_files_updated"]["dot_env"] is True
    assert body["config_files_updated"]["default_yaml"] is True


def test_post_execution_profile_requires_key_when_configured(monkeypatch):
    monkeypatch.setenv("NM_EXECUTION_PROFILE_LEGACY_API", "true")
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key="k"))
    client = TestClient(api.app)
    r = client.post("/system/execution-profile", json={"execution_mode": "live"})
    assert r.status_code == 401
    r2 = client.post(
        "/system/execution-profile",
        json={"execution_mode": "live"},
        headers={"X-API-Key": "k"},
    )
    assert r2.status_code == 200


def test_post_execution_profile_gone_when_legacy_disabled(monkeypatch):
    monkeypatch.delenv("NM_EXECUTION_PROFILE_LEGACY_API", raising=False)
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    client = TestClient(api.app)
    r = client.post("/system/execution-profile", json={"execution_mode": "live"})
    assert r.status_code == 410
