"""GET/PUT/verify /auth/venue-credentials (FB-UX-006)."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient
from pydantic import SecretStr

from app.config.settings import AppSettings
from app.runtime import venue_credentials_verify as verify_mod
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


def test_verify_venue_credentials_ok_and_diagnoses_field(client_venue, monkeypatch):
    """POST /auth/venue-credentials/verify live-pings venues and reports which field is wrong."""
    c = client_venue
    c.post("/auth/register", json={"email": "v@v.co", "password": "password-88"})
    c.post("/auth/login", json={"email": "v@v.co", "password": "password-88"})

    async def fake_verify_alpaca(api_key, api_secret):
        if api_key == "GOODKEY00000000000" and api_secret == "good-secret-1234567890123456":
            return verify_mod.VerifyResult(ok=True)
        return verify_mod.VerifyResult(ok=False, secret_error="Doesn't look like a valid Alpaca secret key")

    async def fake_verify_coinbase(api_key, api_secret):
        return verify_mod.VerifyResult(ok=True)

    monkeypatch.setattr(verify_mod, "verify_alpaca", fake_verify_alpaca)
    monkeypatch.setattr(verify_mod, "verify_coinbase", fake_verify_coinbase)

    # Wrong secret → diagnosed and attributed to the secret field, nothing persisted as verified-good.
    bad = c.post(
        "/auth/venue-credentials/verify",
        json={"alpaca_api_key": "GOODKEY00000000000", "alpaca_api_secret": "short"},
    )
    assert bad.status_code == 200
    bj = bad.json()
    assert bj["alpaca"]["ok"] is False
    assert bj["alpaca"]["key_error"] is None
    assert "secret" in bj["alpaca"]["secret_error"].lower()
    assert bj["coinbase"] is None  # not provided → not checked

    # Correct pair → ok.
    good = c.post(
        "/auth/venue-credentials/verify",
        json={"alpaca_api_key": "GOODKEY00000000000", "alpaca_api_secret": "good-secret-1234567890123456"},
    )
    assert good.status_code == 200
    gj = good.json()
    assert gj["alpaca"]["ok"] is True
    assert gj["alpaca"]["key_error"] is None
    assert gj["alpaca"]["secret_error"] is None


def test_verify_venue_credentials_skips_untouched_incomplete_pairs(client_venue, monkeypatch):
    """A venue with no stored creds and only one fresh field can't form a pair — skipped."""
    c = client_venue
    c.post("/auth/register", json={"email": "w@v.co", "password": "password-88"})
    c.post("/auth/login", json={"email": "w@v.co", "password": "password-88"})

    called = {"alpaca": False, "coinbase": False}

    async def fake_verify_alpaca(api_key, api_secret):
        called["alpaca"] = True
        return verify_mod.VerifyResult(ok=True)

    async def fake_verify_coinbase(api_key, api_secret):
        called["coinbase"] = True
        return verify_mod.VerifyResult(ok=True)

    monkeypatch.setattr(verify_mod, "verify_alpaca", fake_verify_alpaca)
    monkeypatch.setattr(verify_mod, "verify_coinbase", fake_verify_coinbase)

    # No stored creds yet, so a lone fresh key has no secret to pair with → can't be verified.
    r = c.post("/auth/venue-credentials/verify", json={"alpaca_api_key": "ONLYKEY00000000000"})
    assert r.status_code == 200
    j = r.json()
    assert j["alpaca"] is None
    assert j["coinbase"] is None
    assert called == {"alpaca": False, "coinbase": False}


def test_verify_venue_credentials_merges_partial_edit_with_stored_value(client_venue, monkeypatch):
    """Editing only one field of an already-saved pair still triggers a full live check.

    Regression coverage for: "I'm already connected with all four keys but then I type
    garbage into just the Alpaca API key — clicking save should still test it and catch
    the bad value," even though the secret field was left as "keep existing" (blank).
    """
    c = client_venue
    c.post("/auth/register", json={"email": "m@v.co", "password": "password-88"})
    c.post("/auth/login", json={"email": "m@v.co", "password": "password-88"})

    # Seed fully-saved, good-looking credentials for both venues.
    c.put(
        "/auth/venue-credentials",
        json={
            "alpaca_api_key": "GOODKEY00000000000",
            "alpaca_api_secret": "good-secret-1234567890123456",
            "coinbase_api_key": "organizations/org-id/apiKeys/key-id",
            "coinbase_api_secret": "-----BEGIN EC PRIVATE KEY-----\nfake\n-----END EC PRIVATE KEY-----",
        },
    )

    seen: list[tuple[str, str]] = []

    async def fake_verify_alpaca(api_key, api_secret):
        seen.append((api_key, api_secret))
        if api_key == "A":
            return verify_mod.VerifyResult(ok=False, key_error="Doesn't look like a valid Alpaca API key")
        return verify_mod.VerifyResult(ok=True)

    async def fake_verify_coinbase(api_key, api_secret):
        return verify_mod.VerifyResult(ok=True)

    monkeypatch.setattr(verify_mod, "verify_alpaca", fake_verify_alpaca)
    monkeypatch.setattr(verify_mod, "verify_coinbase", fake_verify_coinbase)

    # Only the Alpaca *key* field is edited (typed garbage "A"); secret left blank ("keep existing").
    r = c.post("/auth/venue-credentials/verify", json={"alpaca_api_key": "A"})
    assert r.status_code == 200
    j = r.json()

    # The verifier was actually invoked, paired with the *stored* secret — not skipped.
    assert seen == [("A", "good-secret-1234567890123456")]
    assert j["alpaca"]["ok"] is False
    assert j["alpaca"]["key_error"] is not None
    assert "key" in j["alpaca"]["key_error"].lower()
    # The untouched secret field must never be blamed.
    assert j["alpaca"]["secret_error"] is None
    # Coinbase wasn't touched at all → not checked.
    assert j["coinbase"] is None
