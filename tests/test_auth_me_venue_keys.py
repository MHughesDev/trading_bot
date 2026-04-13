"""GET /auth/me and login/register include venue_keys_* when gating may apply (FB-UX-015)."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient
from pydantic import SecretStr

from app.config.settings import AppSettings
from control_plane import api


@pytest.fixture
def client_auth_venue(tmp_path, monkeypatch):
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


def test_me_without_gate_returns_false_required(client_auth_venue: TestClient) -> None:
    c = client_auth_venue
    c.post("/auth/register", json={"email": "a@b.co", "password": "password-88"})
    c.post("/auth/login", json={"email": "a@b.co", "password": "password-88"})
    r = c.get("/auth/me")
    assert r.status_code == 200
    j = r.json()
    assert j.get("venue_keys_required") is False
    assert j.get("venue_keys_complete") is True


def test_me_with_gate_paper_incomplete(client_auth_venue: TestClient, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_STREAMLIT_VENUE_KEYS_REQUIRED", "true")
    c = client_auth_venue
    c.post("/auth/register", json={"email": "x@y.co", "password": "password-88"})
    c.post("/auth/login", json={"email": "x@y.co", "password": "password-88"})
    r = c.get("/auth/me")
    assert r.status_code == 200
    j = r.json()
    assert j.get("venue_keys_required") is True
    assert j.get("venue_keys_complete") is False


def test_me_with_gate_paper_complete(client_auth_venue: TestClient, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_STREAMLIT_VENUE_KEYS_REQUIRED", "true")
    c = client_auth_venue
    c.post("/auth/register", json={"email": "p@q.co", "password": "password-88"})
    c.post("/auth/login", json={"email": "p@q.co", "password": "password-88"})
    c.put(
        "/auth/venue-credentials",
        json={"alpaca_api_key": "PK1", "alpaca_api_secret": "SEC1"},
    )
    r = c.get("/auth/me")
    j = r.json()
    assert j.get("venue_keys_required") is True
    assert j.get("venue_keys_complete") is True


def test_register_response_includes_venue_fields(client_auth_venue: TestClient, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_STREAMLIT_VENUE_KEYS_REQUIRED", "true")
    c = client_auth_venue
    r = c.post("/auth/register", json={"email": "new@u.co", "password": "password-88"})
    assert r.status_code == 200
    j = r.json()
    assert j.get("venue_keys_required") is True
    assert j.get("venue_keys_complete") is False
