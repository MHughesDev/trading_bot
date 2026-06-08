"""Per-user venue credential encryption (FB-UX-006)."""

from __future__ import annotations

from pathlib import Path

import pytest

from app.runtime import user_store as us
from app.runtime import user_venue_credentials as uvc


@pytest.fixture
def db_with_user(tmp_path: Path) -> tuple[Path, int]:
    db = tmp_path / "u.sqlite"
    r = us.create_user(db, "v@x.co", "password-88")
    return db, r.id


def test_save_load_masked(db_with_user: tuple[Path, int]) -> None:
    db, uid = db_with_user
    master = "test-master-secret-for-fernet-key-derivation"
    uvc.save_credentials(
        db,
        master,
        uid,
        alpaca_api_key="pk-alpaca-xx",
        alpaca_api_secret="secret-alpaca-yy",
        coinbase_api_key="cb-key",
        coinbase_api_secret="cb-secret",
    )
    m = uvc.load_masked(db, master, uid)
    assert m["alpaca_key_set"] is True
    assert m["alpaca_secret_set"] is True
    assert m["coinbase_key_set"] is True
    assert m["coinbase_secret_masked"] == "****cret"
    assert m["alpaca_key_masked"] == "****a-xx"


def test_clear_alpaca(db_with_user: tuple[Path, int]) -> None:
    db, uid = db_with_user
    master = "x" * 32
    uvc.save_credentials(db, master, uid, alpaca_api_key="k", alpaca_api_secret="s")
    uvc.save_credentials(db, master, uid, clear_alpaca=True)
    m = uvc.load_masked(db, master, uid)
    assert m["alpaca_key_set"] is False
    assert m["alpaca_secret_set"] is False
