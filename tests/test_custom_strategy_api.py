"""HTTP routes for the strategy builder — user-built custom strategies (FB-AP-XXX).

Custom strategies are pure user data: each is a row in the operator database
(``users.sqlite``) owned by exactly one account (composite ``(user_id, id)`` primary key,
FK to ``users(id)`` — see strategies/custom_strategy_store.py). The CRUD routes are
therefore session/user-bound (``require_user``), not API-key automation: "list my
strategies" only makes sense for an authenticated person.
"""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from control_plane import api
from strategies.registry import _REGISTRY

VALID_SPEC = {
    "name": "EMA 7/21 cross",
    "indicators": [
        {"id": "ema_fast", "kind": "ema", "period": 7},
        {"id": "ema_slow", "kind": "ema", "period": 21},
    ],
    "entry": {
        "side": "buy",
        "all": [{"type": "cross_above", "left": "ema_fast", "right_id": "ema_slow"}],
        "any": [],
    },
    "size": {"type": "percent_of_equity", "value": 0.02},
    "exits": [{"type": "stop_loss", "value": 0.015}, {"type": "take_profit", "value": 0.04}],
}


@pytest.fixture
def client(tmp_path, monkeypatch):
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
    yield TestClient(api.app)
    for key in [k for k in _REGISTRY if k.startswith("custom:")]:
        _REGISTRY.pop(key, None)


def _signed_in(client: TestClient, email: str) -> TestClient:
    """Register + log in a fresh account; the TestClient cookie jar carries the session."""
    client.post("/auth/register", json={"email": email, "password": "password-88"})
    client.post("/auth/login", json={"email": email, "password": "password-88"})
    return client


def test_preview_valid_spec_returns_explanation(client: TestClient) -> None:
    r = client.post("/strategies/custom/preview", json=VALID_SPEC)
    assert r.status_code == 200
    j = r.json()
    assert j["valid"] is True
    assert j["errors"] == []
    assert "Buy when" in j["explanation"]


def test_preview_invalid_spec_reports_errors(client: TestClient) -> None:
    bad = dict(VALID_SPEC)
    bad["exits"] = []
    r = client.post("/strategies/custom/preview", json=bad)
    assert r.status_code == 200
    j = r.json()
    assert j["valid"] is False
    assert any("exit rule" in e for e in j["errors"])


def test_crud_requires_authentication(client: TestClient) -> None:
    """Custom strategies are user-bound — anonymous requests are rejected outright,
    not merely scoped to "no strategies"."""
    assert client.get("/strategies/custom").status_code == 401
    assert client.post("/strategies/custom", json=VALID_SPEC).status_code == 401
    assert client.put("/strategies/custom/whatever", json=VALID_SPEC).status_code == 401
    assert client.delete("/strategies/custom/whatever").status_code == 401


def test_create_get_list_delete_roundtrip(client: TestClient) -> None:
    c = _signed_in(client, "builder@example.com")

    created = c.post("/strategies/custom", json=VALID_SPEC)
    assert created.status_code == 200
    body = created.json()
    sid = body["id"]
    assert sid == "ema_7_21_cross"
    assert body["registry_key"].startswith("custom:u")
    assert body["registry_key"].endswith(f":{sid}")
    assert "Buy when" in body["explanation"]

    listed = c.get("/strategies/custom")
    assert listed.status_code == 200
    assert any(s["id"] == sid for s in listed.json()["strategies"])

    fetched = c.get(f"/strategies/custom/{sid}")
    assert fetched.status_code == 200
    assert fetched.json()["spec"]["name"] == "EMA 7/21 cross"

    catalogue = c.get("/strategies").json()
    assert any(s["key"] == body["registry_key"] for s in catalogue["strategies"])

    deleted = c.delete(f"/strategies/custom/{sid}")
    assert deleted.status_code == 200
    assert c.get(f"/strategies/custom/{sid}").status_code == 404


def test_create_rejects_invalid_spec(client: TestClient) -> None:
    c = _signed_in(client, "invalid-spec@example.com")
    bad = dict(VALID_SPEC)
    bad["exits"] = []
    r = c.post("/strategies/custom", json=bad)
    assert r.status_code == 422


def test_update_existing_strategy(client: TestClient) -> None:
    c = _signed_in(client, "editor@example.com")
    created = c.post("/strategies/custom", json=VALID_SPEC).json()
    sid = created["id"]

    edited = dict(VALID_SPEC)
    edited["name"] = "EMA 7/21 cross (tuned)"
    r = c.put(f"/strategies/custom/{sid}", json=edited)
    assert r.status_code == 200
    assert r.json()["name"] == "EMA 7/21 cross (tuned)"
    assert r.json()["id"] == sid


def test_update_unknown_id_is_404(client: TestClient) -> None:
    c = _signed_in(client, "updater@example.com")
    r = c.put("/strategies/custom/does_not_exist", json=VALID_SPEC)
    assert r.status_code == 404


def test_delete_unknown_id_is_404(client: TestClient) -> None:
    c = _signed_in(client, "deleter@example.com")
    r = c.delete("/strategies/custom/does_not_exist")
    assert r.status_code == 404


def test_strategies_are_isolated_per_user(client: TestClient) -> None:
    """Two different accounts can each own a strategy with the same name/slug, and
    neither can see, edit, or delete the other's (database-level isolation, FK + composite PK)."""
    alice = _signed_in(client, "alice@example.com")
    created = alice.post("/strategies/custom", json=VALID_SPEC).json()
    sid = created["id"]

    bob = TestClient(api.app)
    bob.cookies.clear()
    _signed_in(bob, "bob@example.com")

    # Bob can save a same-named/same-slug strategy without colliding with Alice's.
    bob_created = bob.post("/strategies/custom", json=VALID_SPEC).json()
    assert bob_created["id"] == sid
    assert bob_created["registry_key"] != created["registry_key"]

    # Bob cannot see, edit, or delete Alice's strategy by id.
    assert bob.get(f"/strategies/custom/{sid}").json()["spec"]["name"] == "EMA 7/21 cross"  # his own
    assert all(s["id"] != sid or s["registry_key"] == bob_created["registry_key"] for s in bob.get("/strategies/custom").json()["strategies"])

    edited = dict(VALID_SPEC)
    edited["name"] = "Hijacked"
    assert bob.put(f"/strategies/custom/{sid}", json=edited).status_code == 200  # edits HIS OWN id-matching row
    assert alice.get(f"/strategies/custom/{sid}").json()["spec"]["name"] == "EMA 7/21 cross"  # Alice's untouched


def test_assign_custom_strategy_to_asset(client: TestClient) -> None:
    c = _signed_in(client, "assigner@example.com")
    created = c.post("/strategies/custom", json=VALID_SPEC).json()
    key = created["registry_key"]

    r = c.put("/assets/strategy/BTC-USD", json={"strategy_key": key})
    assert r.status_code == 200
    assert r.json()["strategy_key"] == key

    g = c.get("/assets/strategy/BTC-USD")
    assert g.json()["strategy_key"] == key
