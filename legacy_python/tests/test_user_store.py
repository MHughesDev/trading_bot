"""SQLite user store (FB-UX-001)."""

from __future__ import annotations

from pathlib import Path

import pytest

from app.runtime import user_store as us


def test_create_and_duplicate(tmp_path: Path) -> None:
    db = tmp_path / "u.sqlite"
    r = us.create_user(db, "A@Example.com", "password-ok-1")
    assert r.id == 1
    assert r.email == "a@example.com"
    with pytest.raises(us.DuplicateEmailError):
        us.create_user(db, "a@example.com", "password-ok-2")


def test_normalize_and_validate() -> None:
    with pytest.raises(us.InvalidEmailError):
        us.normalize_email("not-an-email")
    with pytest.raises(us.InvalidPasswordError):
        us.validate_password("short")


def test_verify_password(tmp_path: Path) -> None:
    db = tmp_path / "u.sqlite"
    us.create_user(db, "x@y.co", "password-ok-1")
    ok = us.verify_password(db, "X@Y.CO", "password-ok-1")
    assert ok is not None
    assert ok.email == "x@y.co"
    assert us.verify_password(db, "x@y.co", "wrong") is None
