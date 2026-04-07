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
