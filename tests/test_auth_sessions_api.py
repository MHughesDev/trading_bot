"""FB-UX-002 session login and mutate authorization."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from control_plane import api


@pytest.fixture
def client_sessions(tmp_path, monkeypatch):
    db = tmp_path / "users.sqlite"
    monkeypatch.setattr(
        api,
        "settings",
        AppSettings(
            auth_users_db_path=db,
            auth_session_enabled=True,
            auth_session_ttl_seconds=3600,
            control_plane_api_key=None,
        ),
    )
    return TestClient(api.app)


def test_login_disabled_when_sessions_off(tmp_path, monkeypatch):
    monkeypatch.setattr(
        api,
        "settings",
        AppSettings(auth_users_db_path=tmp_path / "u.sqlite", auth_session_enabled=False),
    )
    c = TestClient(api.app)
    r = c.post("/auth/login", json={"email": "a@b.co", "password": "password-88"})
    assert r.status_code == 403


def test_login_logout_and_me(client_sessions: TestClient) -> None:
    r = client_sessions.post("/auth/register", json={"email": "u@x.co", "password": "password-88"})
    assert r.status_code == 200
    r = client_sessions.post("/auth/login", json={"email": "u@x.co", "password": "password-88"})
    assert r.status_code == 200
    me = client_sessions.get("/auth/me")
    assert me.status_code == 200
    assert me.json()["email"] == "u@x.co"
    out = client_sessions.post("/auth/logout")
    assert out.status_code == 200
    assert client_sessions.get("/auth/me").status_code == 401


def test_mutate_with_session_cookie_no_api_key(client_sessions: TestClient) -> None:
    client_sessions.post("/auth/register", json={"email": "a@b.co", "password": "password-88"})
    client_sessions.post("/auth/login", json={"email": "a@b.co", "password": "password-88"})
    r = client_sessions.post("/params", json={"x": 1})
    assert r.status_code == 200


def test_mutate_rejects_without_session_when_key_required(tmp_path, monkeypatch):
    monkeypatch.setattr(
        api,
        "settings",
        AppSettings(
            auth_users_db_path=tmp_path / "u.sqlite",
            auth_session_enabled=True,
            control_plane_api_key="secret-k",
        ),
    )
    c = TestClient(api.app)
    assert c.post("/params", json={"x": 1}).status_code == 401
    assert c.post("/params", json={"x": 1}, headers={"X-API-Key": "secret-k"}).status_code == 200
