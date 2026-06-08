"""FB-CAN-038: governance shadow comparison endpoints."""

from __future__ import annotations

from unittest.mock import patch

from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from control_plane import api


def test_get_shadow_comparison() -> None:
    c = TestClient(api.app)
    r = c.get("/governance/shadow-comparison")
    assert r.status_code == 200
    data = r.json()
    assert "policy" in data
    assert data["policy"].get("enabled") is True


def test_post_shadow_comparison_run_requires_key(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(
        api,
        "settings",
        AppSettings(
            auth_users_db_path=tmp_path / "users.sqlite",
            auth_session_enabled=False,
            control_plane_api_key="k",
        ),
    )
    c = TestClient(api.app)
    r = c.post("/governance/shadow-comparison/run", json={"bars": 200})
    assert r.status_code == 401
    with patch("control_plane.api.save_shadow_comparison_report"):
        r2 = c.post(
            "/governance/shadow-comparison/run",
            json={"bars": 200},
            headers={"X-API-Key": "k"},
        )
    assert r2.status_code == 200
    body = r2.json()
    assert "rates" in body
    assert "shadow_comparison_passed" in body
