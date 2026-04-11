from pydantic import SecretStr

from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from control_plane import api


def test_mutate_requires_api_key_when_configured(monkeypatch):
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key="secret-key"))
    client = TestClient(api.app)
    r = client.post("/params", json={"x": 1})
    assert r.status_code == 401
    r2 = client.post("/params", json={"x": 1}, headers={"X-API-Key": "secret-key"})
    assert r2.status_code == 200


def test_mutate_open_when_no_key(monkeypatch):
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    client = TestClient(api.app)
    r = client.post("/params", json={"x": 1})
    assert r.status_code == 200


def test_status_production_preflight_ok_paper_with_keys(monkeypatch):
    monkeypatch.setattr(
        api,
        "settings",
        AppSettings(
            execution_mode="paper",
            risk_signing_secret=SecretStr("x" * 32),
            allow_unsigned_execution=False,
            alpaca_api_key=SecretStr("k"),
            alpaca_api_secret=SecretStr("s"),
        ),
    )
    client = TestClient(api.app)
    r = client.get("/status")
    assert r.status_code == 200
    pf = r.json()["production_preflight"]
    assert pf["ok"] is True
    assert pf["signing_secret_configured"] is True
    assert pf["unsigned_execution_allowed"] is False
    assert pf["venue_credentials_configured"] is True


def test_status_production_preflight_not_ok_live_without_coinbase(monkeypatch):
    monkeypatch.setattr(
        api,
        "settings",
        AppSettings(
            execution_mode="live",
            risk_signing_secret=SecretStr("x" * 32),
            allow_unsigned_execution=False,
        ),
    )
    client = TestClient(api.app)
    r = client.get("/status")
    pf = r.json()["production_preflight"]
    assert pf["ok"] is False
    assert pf["venue_credentials_configured"] is False
