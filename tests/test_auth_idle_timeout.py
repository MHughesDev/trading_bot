"""FB-AUTH-001 idle session timeout behavior."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from pathlib import Path

from app.runtime import operator_sessions as osess


def _set_last_activity(db: Path, token: str, when: datetime) -> None:
    ts = when.astimezone(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    osess.ensure_sessions_schema(db)
    with osess._connect(db) as conn:  # type: ignore[attr-defined]
        conn.execute("UPDATE operator_sessions SET last_activity_at = ? WHERE token = ?", (ts, token))
        conn.commit()


def test_idle_timeout_invalidates_stale_session(tmp_path: Path) -> None:
    db = tmp_path / "users.sqlite"
    sess = osess.create_session(db, user_id=7, ttl_seconds=3600)
    _set_last_activity(db, sess.token, datetime.now(UTC) - timedelta(seconds=7201))
    uid = osess.resolve_session_user_id(db, sess.token, idle_timeout_seconds=7200)
    assert uid is None


def test_idle_timeout_allows_fresh_session(tmp_path: Path) -> None:
    db = tmp_path / "users.sqlite"
    sess = osess.create_session(db, user_id=8, ttl_seconds=3600)
    _set_last_activity(db, sess.token, datetime.now(UTC) - timedelta(seconds=60))
    uid = osess.resolve_session_user_id(db, sess.token, idle_timeout_seconds=7200)
    assert uid == 8


def test_idle_timeout_boundary_exact_7200_is_allowed(tmp_path: Path) -> None:
    db = tmp_path / "users.sqlite"
    sess = osess.create_session(db, user_id=9, ttl_seconds=3600)
    _set_last_activity(db, sess.token, datetime.now(UTC) - timedelta(seconds=7200))
    uid = osess.resolve_session_user_id(db, sess.token, idle_timeout_seconds=7200)
    assert uid == 9
