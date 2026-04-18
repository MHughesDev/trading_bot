"""FB-UX-017 wizard resume + backend non-clobber checks."""

from __future__ import annotations

from pathlib import Path

import pytest
pytest.importorskip("fastapi")
from fastapi.testclient import TestClient
from pydantic import SecretStr

from app.config.settings import AppSettings
from control_plane import api


SRC = Path("control_plane/pages/98_Setup_API_keys.py").read_text(encoding="utf-8")


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


def test_resume_logic_mentions_alpaca_then_coinbase_and_done_redirect() -> None:
    assert "def _current_step" in SRC
    assert "if alpaca_ok and coinbase_ok:" in SRC
    assert 'return "done"' in SRC
    assert "if alpaca_ok and not coinbase_ok:" in SRC
    assert 'return "coinbase"' in SRC


def test_masked_prefill_and_partial_save_path_present() -> None:
    assert "_prefill_mask" in SRC
    assert "_resolve_input" in SRC
    assert 'api_put_json("/auth/venue-credentials", body, require_key=False)' in SRC


def test_backend_partial_update_non_clobber(client_venue: TestClient) -> None:
    c = client_venue
    c.post("/auth/register", json={"email": "keep@keys.co", "password": "password-88"})
    c.post("/auth/login", json={"email": "keep@keys.co", "password": "password-88"})

    p1 = c.put(
        "/auth/venue-credentials",
        json={
            "alpaca_api_key": "AK-1111",
            "alpaca_api_secret": "AS-1111",
        },
    )
    assert p1.status_code == 200
    assert p1.json()["alpaca_key_set"] is True
    assert p1.json()["coinbase_key_set"] is False

    p2 = c.put(
        "/auth/venue-credentials",
        json={
            "coinbase_api_key": "CK-2222",
            "coinbase_api_secret": "CS-2222",
        },
    )
    assert p2.status_code == 200

    g = c.get("/auth/venue-credentials")
    assert g.status_code == 200
    j = g.json()
    assert j["alpaca_key_set"] is True
    assert j["alpaca_secret_set"] is True
    assert j["coinbase_key_set"] is True
    assert j["coinbase_secret_set"] is True
