"""FB-AUD-013: mutate routes reject wrong/missing API key when NM_CONTROL_PLANE_API_KEY is set."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from control_plane import api


@pytest.fixture
def client_with_api_key(tmp_path, monkeypatch):
    monkeypatch.setattr(
        api,
        "settings",
        AppSettings(
            auth_users_db_path=tmp_path / "users.sqlite",
            auth_session_enabled=False,
            control_plane_api_key="correct-secret-key",
        ),
    )
    return TestClient(api.app)


def test_mutate_rejects_missing_key(client_with_api_key: TestClient) -> None:
    r = client_with_api_key.post("/params", json={"x": 1})
    assert r.status_code == 401


def test_mutate_rejects_wrong_key(client_with_api_key: TestClient) -> None:
    r = client_with_api_key.post(
        "/params",
        json={"x": 1},
        headers={"X-API-Key": "wrong"},
    )
    assert r.status_code == 401


def test_mutate_accepts_correct_key(client_with_api_key: TestClient) -> None:
    r = client_with_api_key.post(
        "/params",
        json={"x": 1},
        headers={"X-API-Key": "correct-secret-key"},
    )
    assert r.status_code == 200
