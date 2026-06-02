"""Webull per-user venue credential storage + API (extends FB-UX-006)."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient
from pydantic import SecretStr

from app.config.settings import AppSettings
from app.runtime import user_venue_credentials as vc
from control_plane import api

_MASTER = "test-venue-master-secret-key-32bytes!"


def test_store_save_load_mask_webull(tmp_path) -> None:
    db = tmp_path / "users.sqlite"
    vc.save_credentials(db, _MASTER, 1, webull_api_key="WBKEY1234", webull_api_secret="WBSEC9876")
    masked = vc.load_masked(db, _MASTER, 1)
    assert masked["webull_key_set"] is True
    assert masked["webull_secret_set"] is True
    assert masked["webull_key_masked"] == "****1234"
    assert masked["webull_secret_masked"] == "****9876"
    dec = vc.load_decrypted_credentials(db, _MASTER, 1)
    assert dec["webull_api_key"] == "WBKEY1234"
    assert dec["webull_api_secret"] == "WBSEC9876"


def test_store_webull_does_not_clobber_alpaca(tmp_path) -> None:
    db = tmp_path / "users.sqlite"
    vc.save_credentials(db, _MASTER, 1, alpaca_api_key="PKABC", alpaca_api_secret="ASEC")
    vc.save_credentials(db, _MASTER, 1, webull_api_key="WBK", webull_api_secret="WBS")
    masked = vc.load_masked(db, _MASTER, 1)
    assert masked["alpaca_key_set"] is True  # preserved across the second save
    assert masked["webull_key_set"] is True


def test_store_clear_webull(tmp_path) -> None:
    db = tmp_path / "users.sqlite"
    vc.save_credentials(db, _MASTER, 1, webull_api_key="WBK", webull_api_secret="WBS")
    vc.save_credentials(db, _MASTER, 1, clear_webull=True)
    masked = vc.load_masked(db, _MASTER, 1)
    assert masked["webull_key_set"] is False
    assert masked["webull_secret_set"] is False


def test_migration_adds_webull_columns_to_old_table(tmp_path) -> None:
    """A pre-Webull table (4 credential columns) is migrated, not dropped."""
    import sqlite3

    db = tmp_path / "users.sqlite"
    conn = sqlite3.connect(str(db))
    conn.execute(
        """CREATE TABLE user_venue_credentials (
            user_id INTEGER PRIMARY KEY,
            alpaca_key_enc BLOB, alpaca_secret_enc BLOB,
            coinbase_key_enc BLOB, coinbase_secret_enc BLOB,
            updated_at TEXT NOT NULL
        )"""
    )
    conn.commit()
    conn.close()
    # Saving Webull keys should migrate + persist without error.
    vc.save_credentials(db, _MASTER, 1, webull_api_key="WBK", webull_api_secret="WBS")
    assert vc.load_masked(db, _MASTER, 1)["webull_key_set"] is True


@pytest.fixture
def client_venue(tmp_path, monkeypatch):
    monkeypatch.setattr(
        api,
        "settings",
        AppSettings(
            auth_users_db_path=tmp_path / "users.sqlite",
            auth_session_enabled=True,
            auth_session_ttl_seconds=3600,
            auth_venue_credentials_master_secret=SecretStr(_MASTER),
            control_plane_api_key=None,
        ),
    )
    return TestClient(api.app)


def test_api_put_get_webull(client_venue: TestClient) -> None:
    c = client_venue
    c.post("/auth/register", json={"email": "w@v.co", "password": "password-88"})
    c.post("/auth/login", json={"email": "w@v.co", "password": "password-88"})
    p = c.put(
        "/auth/venue-credentials",
        json={"webull_api_key": "WBKEY1234", "webull_api_secret": "WBSEC9876"},
    )
    assert p.status_code == 200
    body = p.json()
    assert body["webull_key_set"] is True
    assert body["webull_key_masked"] == "****1234"
    g = c.get("/auth/venue-credentials")
    assert g.json()["webull_secret_masked"] == "****9876"


def test_api_webull_does_not_clobber_alpaca(client_venue: TestClient) -> None:
    c = client_venue
    c.post("/auth/register", json={"email": "w2@v.co", "password": "password-88"})
    c.post("/auth/login", json={"email": "w2@v.co", "password": "password-88"})
    c.put("/auth/venue-credentials", json={"alpaca_api_key": "PKABC", "alpaca_api_secret": "ASEC"})
    c.put("/auth/venue-credentials", json={"webull_api_key": "WBK", "webull_api_secret": "WBS"})
    g = c.get("/auth/venue-credentials").json()
    assert g["alpaca_key_set"] is True
    assert g["webull_key_set"] is True
