"""Control plane system power endpoints."""

from __future__ import annotations

from pathlib import Path

from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.runtime import system_power as sp
from control_plane import api


def test_get_system_power_default(monkeypatch, tmp_path: Path):
    monkeypatch.setenv("NM_SYSTEM_POWER_LEGACY_ENABLED", "true")
    monkeypatch.setattr(sp, "_STATE_PATH", tmp_path / "system_power.json")
    sp.set_power("on")
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    client = TestClient(api.app)
    r = client.get("/system/power")
    assert r.status_code == 200
    assert r.json()["power"] == "on"
    assert r.json().get("legacy_enabled") is True


def test_post_system_power_persists(monkeypatch, tmp_path: Path):
    monkeypatch.setenv("NM_SYSTEM_POWER_LEGACY_ENABLED", "true")
    state_file = tmp_path / "system_power.json"
    monkeypatch.setattr(sp, "_STATE_PATH", state_file)
    sp.set_power("on")
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    client = TestClient(api.app)
    r = client.post("/system/power", json={"power": "off"})
    assert r.status_code == 200
    assert r.json() == {"power": "off"}
    assert sp.get_power() == "off"
    r2 = client.get("/status")
    assert r2.status_code == 200
    assert r2.json()["system_power"] == "off"


def test_post_system_power_requires_key_when_configured(monkeypatch, tmp_path: Path):
    monkeypatch.setenv("NM_SYSTEM_POWER_LEGACY_ENABLED", "true")
    monkeypatch.setattr(sp, "_STATE_PATH", tmp_path / "system_power.json")
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key="k"))
    client = TestClient(api.app)
    r = client.post("/system/power", json={"power": "on"})
    assert r.status_code == 401
    r2 = client.post("/system/power", json={"power": "on"}, headers={"X-API-Key": "k"})
    assert r2.status_code == 200


def test_post_system_power_gone_when_legacy_disabled(monkeypatch, tmp_path: Path):
    monkeypatch.delenv("NM_SYSTEM_POWER_LEGACY_ENABLED", raising=False)
    monkeypatch.setattr(sp, "_STATE_PATH", tmp_path / "system_power.json")
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    client = TestClient(api.app)
    r = client.post("/system/power", json={"power": "off"})
    assert r.status_code == 410
