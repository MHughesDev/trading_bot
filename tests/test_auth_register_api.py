"""POST /auth/register (FB-UX-001)."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from control_plane import api


@pytest.fixture
def client(tmp_path, monkeypatch):
    db = tmp_path / "users.sqlite"
    monkeypatch.setattr(api, "settings", AppSettings(auth_users_db_path=db))
    return TestClient(api.app)


def test_register_ok(client: TestClient) -> None:
    r = client.post("/auth/register", json={"email": "Op@Example.com", "password": "secret-88"})
    assert r.status_code == 200
    j = r.json()
    assert j["email"] == "op@example.com"
    assert j["id"] == 1
    assert "created_at" in j


def test_register_duplicate(client: TestClient) -> None:
    client.post("/auth/register", json={"email": "a@b.co", "password": "password-88"})
    r = client.post("/auth/register", json={"email": "A@B.CO", "password": "password-99"})
    assert r.status_code == 409


def test_register_bad_email(client: TestClient) -> None:
    r = client.post("/auth/register", json={"email": "nope", "password": "password-88"})
    assert r.status_code == 422


def test_status_includes_user_store(client: TestClient) -> None:
    r = client.get("/status")
    assert r.status_code == 200
    us = r.json()["user_store"]
    assert us["user_count"] == 0
    assert us["db_path"].endswith("users.sqlite")
