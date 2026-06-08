"""HTTP routes for per-asset execution mode (FB-AP-030)."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.runtime import asset_execution_mode as em
from control_plane import api


@pytest.fixture
def dirs(tmp_path, monkeypatch):
    monkeypatch.setattr(em, "_DEFAULT_DIR", tmp_path / "exec_mode")


@pytest.fixture
def client_no_auth(dirs, monkeypatch):
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    return TestClient(api.app)


@pytest.fixture
def client_with_key(dirs, monkeypatch):
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key="secret-key"))
    return TestClient(api.app)


def test_get_execution_mode_read_only(client_no_auth: TestClient) -> None:
    r = client_no_auth.get("/assets/execution-mode/BTC-USD")
    assert r.status_code == 200
    j = r.json()
    assert j["symbol"] == "BTC-USD"
    assert j["execution_mode"] in ("paper", "live")
    assert j.get("override") is None


def test_put_and_delete_with_key(client_with_key: TestClient) -> None:
    r = client_with_key.put(
        "/assets/execution-mode/BTC-USD",
        json={"execution_mode": "live"},
        headers={"X-API-Key": "secret-key"},
    )
    assert r.status_code == 200
    assert r.json()["execution_mode"] == "live"
    g = client_with_key.get("/assets/execution-mode/BTC-USD")
    assert g.json()["override"] == "live"
    d = client_with_key.delete(
        "/assets/execution-mode/BTC-USD",
        headers={"X-API-Key": "secret-key"},
    )
    assert d.status_code == 200
    assert client_with_key.get("/assets/execution-mode/BTC-USD").json().get("override") is None


def test_status_includes_asset_execution_mode(client_no_auth: TestClient) -> None:
    r = client_no_auth.get("/status")
    assert r.status_code == 200
    assert "asset_execution_mode" in r.json()
    assert "overrides" in r.json()["asset_execution_mode"]
