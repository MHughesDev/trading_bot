"""Per-asset lifecycle API (FB-AP-005)."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.contracts.asset_model_manifest import AssetModelManifest
from app.runtime import asset_lifecycle as life
from app.runtime import asset_model_registry as reg
from control_plane import api


@pytest.fixture
def client_no_auth(tmp_path, monkeypatch):
    manifests = tmp_path / "manifests"
    lifecycle = tmp_path / "lifecycle"
    manifests.mkdir(parents=True)
    monkeypatch.setattr(reg, "_DEFAULT_DIR", manifests)
    monkeypatch.setattr(life, "_DEFAULT_LIFECYCLE_DIR", lifecycle)
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    return TestClient(api.app)


def test_effective_state_manifest_only_no_file(client_no_auth: TestClient, tmp_path) -> None:
    """Manifest without lifecycle row → effective initialized_not_active."""
    reg.save_manifest(
        AssetModelManifest(
            canonical_symbol="ETH-USD",
            forecaster_weights_path="/x/f.npz",
        )
    )
    r = client_no_auth.get("/assets/lifecycle/ETH-USD")
    assert r.status_code == 200
    assert r.json()["state"] == "initialized_not_active"
    assert r.json()["persisted"] is False


def test_initialize_start_stop_flow(client_no_auth: TestClient) -> None:
    r0 = client_no_auth.get("/assets/lifecycle/BTC-USD")
    assert r0.json()["state"] == "uninitialized"

    r1 = client_no_auth.post("/assets/lifecycle/BTC-USD/initialize")
    assert r1.status_code == 200
    assert r1.json()["state"] == "initialized_not_active"

    r_put = client_no_auth.put(
        "/assets/models/BTC-USD",
        json={"canonical_symbol": "BTC-USD", "forecaster_weights_path": "/w/f.npz"},
    )
    assert r_put.status_code == 200

    r2 = client_no_auth.post("/assets/lifecycle/BTC-USD/start")
    assert r2.status_code == 200
    assert r2.json()["state"] == "active"

    r3 = client_no_auth.post("/assets/lifecycle/BTC-USD/stop")
    assert r3.status_code == 200
    assert r3.json()["state"] == "initialized_not_active"


def test_start_requires_manifest(client_no_auth: TestClient) -> None:
    client_no_auth.post("/assets/lifecycle/SOL-USD/initialize")
    r = client_no_auth.post("/assets/lifecycle/SOL-USD/start")
    assert r.status_code == 400
    assert "manifest" in r.json()["detail"].lower()


def test_status_includes_asset_lifecycle(client_no_auth: TestClient) -> None:
    r = client_no_auth.get("/status")
    assert r.status_code == 200
    al = r.json()["asset_lifecycle"]
    assert "lifecycle_dir" in al
    assert "tracked_symbols" in al


def test_illegal_transitions(client_no_auth: TestClient) -> None:
    client_no_auth.post("/assets/lifecycle/X-USD/initialize")
    assert client_no_auth.post("/assets/lifecycle/X-USD/initialize").status_code == 409
    reg.save_manifest(AssetModelManifest(canonical_symbol="X-USD"))
    assert client_no_auth.post("/assets/lifecycle/X-USD/start").status_code == 200
    assert client_no_auth.post("/assets/lifecycle/X-USD/start").status_code == 409
    assert client_no_auth.post("/assets/lifecycle/X-USD/stop").status_code == 200
    assert client_no_auth.post("/assets/lifecycle/X-USD/stop").status_code == 409
