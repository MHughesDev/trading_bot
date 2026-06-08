"""Control plane routes for per-asset lifecycle (FB-AP-005)."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.runtime import asset_lifecycle_state as lc
from app.runtime import asset_model_registry as reg
from control_plane import api


@pytest.fixture
def dirs(tmp_path, monkeypatch):
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path / "manifests")
    monkeypatch.setattr(lc, "_DEFAULT_DIR", tmp_path / "lifecycle")
    (tmp_path / "manifests").mkdir(parents=True, exist_ok=True)


@pytest.fixture
def client_no_auth(dirs, monkeypatch):
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    return TestClient(api.app)


@pytest.fixture
def client_with_key(dirs, monkeypatch):
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key="secret-key"))
    return TestClient(api.app)


def test_get_lifecycle_uninitialized(client_no_auth: TestClient) -> None:
    r = client_no_auth.get("/assets/lifecycle/BTC-USD")
    assert r.status_code == 200
    assert r.json() == {"symbol": "BTC-USD", "lifecycle_state": "uninitialized"}


def test_status_includes_asset_lifecycle(client_no_auth: TestClient) -> None:
    r = client_no_auth.get("/status")
    assert r.status_code == 200
    assert "asset_lifecycle" in r.json()
    assert "states" in r.json()["asset_lifecycle"]


def test_start_stop_with_mutate_key(client_with_key: TestClient, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        api,
        "flatten_symbol_position_sync",
        lambda *_a, **_k: {
            "submitted": False,
            "skipped": "flat",
            "error": None,
            "acks": [],
            "lifecycle_continue": True,
        },
    )
    client_with_key.put(
        "/assets/models/BTC-USD",
        json={"canonical_symbol": "BTC-USD", "forecaster_torch_path": "/x.pt"},
        headers={"X-API-Key": "secret-key"},
    )
    assert client_with_key.get("/assets/lifecycle/BTC-USD").json()["lifecycle_state"] == (
        "initialized_not_active"
    )
    r = client_with_key.post(
        "/assets/lifecycle/BTC-USD/start",
        headers={"X-API-Key": "secret-key"},
    )
    assert r.status_code == 200
    assert r.json()["lifecycle_state"] == "active"
    r2 = client_with_key.post(
        "/assets/lifecycle/BTC-USD/stop",
        headers={"X-API-Key": "secret-key"},
    )
    assert r2.status_code == 200
    assert r2.json()["lifecycle_state"] == "initialized_not_active"


def test_stop_returns_502_when_flatten_fails(
    client_with_key: TestClient, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        api,
        "flatten_symbol_position_sync",
        lambda *_a, **_k: {
            "submitted": False,
            "skipped": "fetch_positions_failed",
            "error": "venue down",
            "acks": [],
            "lifecycle_continue": False,
        },
    )
    client_with_key.put(
        "/assets/models/BTC-USD",
        json={"canonical_symbol": "BTC-USD"},
        headers={"X-API-Key": "secret-key"},
    )
    client_with_key.post(
        "/assets/lifecycle/BTC-USD/start",
        headers={"X-API-Key": "secret-key"},
    )
    r = client_with_key.post(
        "/assets/lifecycle/BTC-USD/stop",
        headers={"X-API-Key": "secret-key"},
    )
    assert r.status_code == 502
    assert client_with_key.get("/assets/lifecycle/BTC-USD").json()["lifecycle_state"] == "active"


def test_delete_manifest_clears_lifecycle_file(client_with_key: TestClient) -> None:
    client_with_key.put(
        "/assets/models/BTC-USD",
        json={"canonical_symbol": "BTC-USD"},
        headers={"X-API-Key": "secret-key"},
    )
    client_with_key.post(
        "/assets/lifecycle/BTC-USD/start",
        headers={"X-API-Key": "secret-key"},
    )
    assert client_with_key.get("/assets/lifecycle/BTC-USD").json()["lifecycle_state"] == "active"
    client_with_key.delete(
        "/assets/models/BTC-USD",
        headers={"X-API-Key": "secret-key"},
    )
    assert client_with_key.get("/assets/lifecycle/BTC-USD").json()["lifecycle_state"] == (
        "uninitialized"
    )
