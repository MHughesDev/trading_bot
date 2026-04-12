"""Operator session store (FB-UX-002)."""

from __future__ import annotations

from pathlib import Path

from app.runtime import operator_sessions as osess


def test_create_resolve_revoke(tmp_path: Path) -> None:
    db = tmp_path / "db.sqlite"
    s = osess.create_session(db, user_id=42, ttl_seconds=3600)
    assert osess.resolve_session_user_id(db, s.token) == 42
    assert osess.revoke_session(db, s.token) is True
    assert osess.resolve_session_user_id(db, s.token) is None


def test_bad_token(tmp_path: Path) -> None:
    db = tmp_path / "db.sqlite"
    assert osess.resolve_session_user_id(db, None) is None
    assert osess.resolve_session_user_id(db, "nope") is None
