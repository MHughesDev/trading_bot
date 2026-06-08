"""Control plane routes for per-asset manifests (FB-AP-002)."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.runtime import asset_model_registry as reg
from control_plane import api


@pytest.fixture
def client_no_auth(tmp_path, monkeypatch):
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path)
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    return TestClient(api.app)


def test_get_assets_models_empty(client_no_auth: TestClient) -> None:
    r = client_no_auth.get("/assets/models")
    assert r.status_code == 200
    assert r.json()["symbols"] == []


def test_put_get_list_delete_manifest(client_no_auth: TestClient) -> None:
    body = {
        "canonical_symbol": "BTC-USD",
        "forecaster_torch_path": "/x/forecaster_torch.pt",
    }
    r = client_no_auth.put("/assets/models/BTC-USD", json=body)
    assert r.status_code == 200
    g = client_no_auth.get("/assets/models/BTC-USD")
    assert g.status_code == 200
    assert g.json()["forecaster_torch_path"] == "/x/forecaster_torch.pt"
    lst = client_no_auth.get("/assets/models")
    assert lst.json()["symbols"] == ["BTC-USD"]
    st = client_no_auth.get("/status")
    assert "BTC-USD" in st.json()["asset_model_registry"]["initialized_symbols"]
    d = client_no_auth.delete("/assets/models/BTC-USD")
    assert d.status_code == 200
    assert client_no_auth.get("/assets/models/BTC-USD").status_code == 404


def test_put_path_symbol_mismatch(client_no_auth: TestClient) -> None:
    r = client_no_auth.put(
        "/assets/models/BTC-USD",
        json={"canonical_symbol": "ETH-USD"},
    )
    assert r.status_code == 422
