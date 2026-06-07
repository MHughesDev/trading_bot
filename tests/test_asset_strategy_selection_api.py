"""HTTP routes for per-asset strategy selection — the live decision source (FB-AP-XXX)."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.runtime import asset_strategy_selection as ass
from control_plane import api


@pytest.fixture
def dirs(tmp_path, monkeypatch):
    monkeypatch.setattr(ass, "_DEFAULT_DIR", tmp_path / "asset_strategy")


@pytest.fixture
def client_no_auth(dirs, monkeypatch):
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    return TestClient(api.app)


@pytest.fixture
def client_with_key(dirs, monkeypatch):
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key="secret-key"))
    return TestClient(api.app)


def test_get_strategy_selection_falls_back_to_default(client_no_auth: TestClient) -> None:
    r = client_no_auth.get("/assets/strategy/BTC-USD")
    assert r.status_code == 200
    j = r.json()
    assert j["symbol"] == "BTC-USD"
    assert j["strategy_key"] == j["default_strategy_key"]
    assert j.get("override") is None


def test_put_unknown_strategy_is_422(client_with_key: TestClient) -> None:
    r = client_with_key.put(
        "/assets/strategy/BTC-USD",
        json={"strategy_key": "not_a_real_strategy"},
        headers={"X-API-Key": "secret-key"},
    )
    assert r.status_code == 422


def test_put_get_delete_roundtrip(client_with_key: TestClient) -> None:
    r = client_with_key.put(
        "/assets/strategy/BTC-USD",
        json={"strategy_key": "ema_cross", "strategy_params": {"fast_ema_period": 5}},
        headers={"X-API-Key": "secret-key"},
    )
    assert r.status_code == 200
    assert r.json()["strategy_key"] == "ema_cross"

    g = client_with_key.get("/assets/strategy/BTC-USD")
    j = g.json()
    assert j["override"] == "ema_cross"
    assert j["strategy_key"] == "ema_cross"
    assert j["strategy_params"] == {"fast_ema_period": 5}

    d = client_with_key.delete("/assets/strategy/BTC-USD", headers={"X-API-Key": "secret-key"})
    assert d.status_code == 200
    assert client_with_key.get("/assets/strategy/BTC-USD").json().get("override") is None


def test_delete_without_override_is_404(client_with_key: TestClient) -> None:
    r = client_with_key.delete("/assets/strategy/BTC-USD", headers={"X-API-Key": "secret-key"})
    assert r.status_code == 404


def test_mutating_routes_require_auth(client_with_key: TestClient) -> None:
    r = client_with_key.put("/assets/strategy/BTC-USD", json={"strategy_key": "ema_cross"})
    assert r.status_code == 401
    r = client_with_key.delete("/assets/strategy/BTC-USD")
    assert r.status_code == 401
