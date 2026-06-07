"""HTTP routes for the strategy builder — user-built custom strategies (FB-AP-XXX)."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from control_plane import api
from strategies import custom_strategy_store as store
from strategies.registry import _REGISTRY

VALID_SPEC = {
    "name": "EMA 7/21 cross",
    "indicators": [
        {"id": "ema_fast", "kind": "ema", "period": 7},
        {"id": "ema_slow", "kind": "ema", "period": 21},
    ],
    "entry": {
        "side": "buy",
        "all": [{"type": "cross_above", "left": "ema_fast", "right_id": "ema_slow"}],
        "any": [],
    },
    "size": {"type": "percent_of_equity", "value": 0.02},
    "exits": [{"type": "stop_loss", "value": 0.015}, {"type": "take_profit", "value": 0.04}],
}


@pytest.fixture
def dirs(tmp_path, monkeypatch):
    monkeypatch.setattr(store, "_DEFAULT_DIR", tmp_path / "custom_strategies")
    yield
    for key in [k for k in _REGISTRY if k.startswith("custom:")]:
        _REGISTRY.pop(key, None)


@pytest.fixture
def client_no_auth(dirs, monkeypatch):
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    return TestClient(api.app)


@pytest.fixture
def client_with_key(dirs, monkeypatch):
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key="secret-key"))
    return TestClient(api.app)


def _auth():
    return {"X-API-Key": "secret-key"}


def test_preview_valid_spec_returns_explanation(client_no_auth: TestClient) -> None:
    r = client_no_auth.post("/strategies/custom/preview", json=VALID_SPEC)
    assert r.status_code == 200
    j = r.json()
    assert j["valid"] is True
    assert j["errors"] == []
    assert "Buy when" in j["explanation"]


def test_preview_invalid_spec_reports_errors(client_no_auth: TestClient) -> None:
    bad = dict(VALID_SPEC)
    bad["exits"] = []
    r = client_no_auth.post("/strategies/custom/preview", json=bad)
    assert r.status_code == 200
    j = r.json()
    assert j["valid"] is False
    assert any("exit rule" in e for e in j["errors"])


def test_create_requires_auth(client_with_key: TestClient) -> None:
    r = client_with_key.post("/strategies/custom", json=VALID_SPEC)
    assert r.status_code == 401


def test_create_get_list_delete_roundtrip(client_with_key: TestClient) -> None:
    created = client_with_key.post("/strategies/custom", json=VALID_SPEC, headers=_auth())
    assert created.status_code == 200
    body = created.json()
    sid = body["id"]
    assert sid == "ema_7_21_cross"
    assert body["registry_key"] == f"custom:{sid}"
    assert "Buy when" in body["explanation"]

    listed = client_with_key.get("/strategies/custom")
    assert listed.status_code == 200
    assert any(s["id"] == sid for s in listed.json()["strategies"])

    fetched = client_with_key.get(f"/strategies/custom/{sid}")
    assert fetched.status_code == 200
    assert fetched.json()["spec"]["name"] == "EMA 7/21 cross"

    catalogue = client_with_key.get("/strategies").json()
    assert any(s["key"] == f"custom:{sid}" for s in catalogue["strategies"])

    deleted = client_with_key.delete(f"/strategies/custom/{sid}", headers=_auth())
    assert deleted.status_code == 200
    assert client_with_key.get(f"/strategies/custom/{sid}").status_code == 404


def test_create_rejects_invalid_spec(client_with_key: TestClient) -> None:
    bad = dict(VALID_SPEC)
    bad["exits"] = []
    r = client_with_key.post("/strategies/custom", json=bad, headers=_auth())
    assert r.status_code == 422


def test_update_existing_strategy(client_with_key: TestClient) -> None:
    created = client_with_key.post("/strategies/custom", json=VALID_SPEC, headers=_auth()).json()
    sid = created["id"]

    edited = dict(VALID_SPEC)
    edited["name"] = "EMA 7/21 cross (tuned)"
    r = client_with_key.put(f"/strategies/custom/{sid}", json=edited, headers=_auth())
    assert r.status_code == 200
    assert r.json()["name"] == "EMA 7/21 cross (tuned)"
    assert r.json()["id"] == sid


def test_update_unknown_id_is_404(client_with_key: TestClient) -> None:
    r = client_with_key.put("/strategies/custom/does_not_exist", json=VALID_SPEC, headers=_auth())
    assert r.status_code == 404


def test_delete_unknown_id_is_404(client_with_key: TestClient) -> None:
    r = client_with_key.delete("/strategies/custom/does_not_exist", headers=_auth())
    assert r.status_code == 404


def test_assign_custom_strategy_to_asset(client_with_key: TestClient) -> None:
    created = client_with_key.post("/strategies/custom", json=VALID_SPEC, headers=_auth()).json()
    key = created["registry_key"]

    r = client_with_key.put("/assets/strategy/BTC-USD", json={"strategy_key": key}, headers=_auth())
    assert r.status_code == 200
    assert r.json()["strategy_key"] == key

    g = client_with_key.get("/assets/strategy/BTC-USD")
    assert g.json()["strategy_key"] == key
