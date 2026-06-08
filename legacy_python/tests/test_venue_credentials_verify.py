"""Format-diagnostic heuristics in app.runtime.venue_credentials_verify (FB-UX-006 hardening).

These exercise the pure pre-flight checks (key/secret shape) that run before any live
network ping, so they're fast and offline. The live-ping branches are covered via
monkeypatching in tests/test_venue_credentials_api.py.
"""

from __future__ import annotations

import pytest

from app.runtime import venue_credentials_verify as verify_mod


class _FakeResponse:
    def __init__(self, status_code: int) -> None:
        self.status_code = status_code


class _FakeAsyncClient:
    def __init__(self, status_code: int) -> None:
        self._status_code = status_code

    async def __aenter__(self):
        return self

    async def __aexit__(self, *exc):
        return False

    async def get(self, *args, **kwargs):
        return _FakeResponse(self._status_code)


@pytest.mark.asyncio
async def test_verify_alpaca_ok(monkeypatch):
    monkeypatch.setattr(verify_mod.httpx, "AsyncClient", lambda **kw: _FakeAsyncClient(200))
    r = await verify_mod.verify_alpaca("PKABCDEFGHIJKLMNOP12", "a" * 40)
    assert r.ok is True
    assert r.key_error is None
    assert r.secret_error is None


@pytest.mark.asyncio
async def test_verify_alpaca_diagnoses_short_secret(monkeypatch):
    monkeypatch.setattr(verify_mod.httpx, "AsyncClient", lambda **kw: _FakeAsyncClient(401))
    r = await verify_mod.verify_alpaca("PKABCDEFGHIJKLMNOP12", "tooshort")
    assert r.ok is False
    assert r.key_error is None
    assert r.secret_error is not None
    assert "secret" in r.secret_error.lower()


@pytest.mark.asyncio
async def test_verify_alpaca_diagnoses_malformed_key(monkeypatch):
    monkeypatch.setattr(verify_mod.httpx, "AsyncClient", lambda **kw: _FakeAsyncClient(401))
    r = await verify_mod.verify_alpaca("not-a-key!!", "a" * 40)
    assert r.ok is False
    assert r.key_error is not None
    assert "key" in r.key_error.lower()
    assert r.secret_error is None


@pytest.mark.asyncio
async def test_verify_alpaca_generic_when_both_look_plausible(monkeypatch):
    monkeypatch.setattr(verify_mod.httpx, "AsyncClient", lambda **kw: _FakeAsyncClient(401))
    r = await verify_mod.verify_alpaca("PKABCDEFGHIJKLMNOP12", "a" * 40)
    assert r.ok is False
    assert r.key_error is None
    assert r.secret_error is None
    assert r.error is not None


@pytest.mark.asyncio
async def test_verify_coinbase_diagnoses_malformed_key_and_secret():
    r = await verify_mod.verify_coinbase("just-some-string", "not-a-pem-block")
    assert r.ok is False
    assert r.key_error is not None and "organizations/" in r.key_error
    assert r.secret_error is not None and "PEM" in r.secret_error


@pytest.mark.asyncio
async def test_verify_coinbase_ok(monkeypatch):
    monkeypatch.setattr(
        verify_mod,
        "jwt_generator",
        type("FakeJwtGen", (), {
            "format_jwt_uri": staticmethod(lambda method, path: f"{method} {path}"),
            "build_rest_jwt": staticmethod(lambda uri, key, secret: "fake.jwt.token"),
        })(),
        raising=False,
    )
    monkeypatch.setattr(verify_mod.httpx, "AsyncClient", lambda **kw: _FakeAsyncClient(200))
    r = await verify_mod.verify_coinbase(
        "organizations/org-id/apiKeys/key-id",
        "-----BEGIN EC PRIVATE KEY-----\nfakefakefake\n-----END EC PRIVATE KEY-----",
    )
    assert r.ok is True
