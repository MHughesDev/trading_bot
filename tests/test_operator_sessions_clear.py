"""clear_all_sessions for the desktop fresh-login-on-launch behavior."""

from __future__ import annotations

from pathlib import Path

from app.runtime import operator_sessions


def test_clear_all_sessions_removes_rows(tmp_path: Path):
    db = tmp_path / "users.sqlite"
    operator_sessions.create_session(db, user_id=1, ttl_seconds=3600)
    operator_sessions.create_session(db, user_id=2, ttl_seconds=3600)

    removed = operator_sessions.clear_all_sessions(db)
    assert removed == 2

    status = operator_sessions.session_status(db)
    assert status.get("active_session_rows", 0) == 0


def test_clear_all_sessions_on_empty_db_is_safe(tmp_path: Path):
    db = tmp_path / "fresh.sqlite"
    # Table does not exist yet; helper must create schema and return 0.
    assert operator_sessions.clear_all_sessions(db) == 0
