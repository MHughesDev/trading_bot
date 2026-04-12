"""GET/PUT /auth/venue-credentials (FB-UX-006)."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient
from pydantic import SecretStr

from app.config.settings import AppSettings
from control_plane import api


@pytest.fixture
def client_venue(tmp_path, monkeypatch):
    db = tmp_path / "users.sqlite"
    monkeypatch.setattr(
        api,
        "settings",
        AppSettings(
            auth_users_db_path=db,
            auth_session_enabled=True,
            auth_session_ttl_seconds=3600,
            auth_venue_credentials_master_secret=SecretStr("test-venue-master-secret-key-32bytes!"),
            control_plane_api_key=None,
        ),
    )
    return TestClient(api.app)


def test_venue_credentials_get_put(client_venue: TestClient) -> None:
    c = client_venue
    c.post("/auth/register", json={"email": "u@v.co", "password": "password-88"})
    c.post("/auth/login", json={"email": "u@v.co", "password": "password-88"})
    g = c.get("/auth/venue-credentials")
    assert g.status_code == 200
    assert g.json()["alpaca_key_set"] is False
    p = c.put(
        "/auth/venue-credentials",
        json={
            "alpaca_api_key": "PKABC",
            "alpaca_api_secret": "SECRET1",
            "coinbase_api_key": "cbk",
            "coinbase_api_secret": "cbs",
        },
    )
    assert p.status_code == 200
    j = p.json()
    assert j["alpaca_key_set"] is True
    assert j["alpaca_key_masked"] == "****KABC"
    g2 = c.get("/auth/venue-credentials")
    assert g2.json()["alpaca_secret_masked"] == "****RET1"


def test_venue_credentials_503_without_master(tmp_path, monkeypatch):
    monkeypatch.setattr(
        api,
        "settings",
        AppSettings(
            auth_users_db_path=tmp_path / "u.sqlite",
            auth_session_enabled=True,
            auth_venue_credentials_master_secret=None,
            control_plane_api_key=None,
        ),
    )
    c = TestClient(api.app)
    c.post("/auth/register", json={"email": "a@b.co", "password": "password-88"})
    c.post("/auth/login", json={"email": "a@b.co", "password": "password-88"})
    assert c.get("/auth/venue-credentials").status_code == 503
