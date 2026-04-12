"""Server-side operator sessions (FB-UX-002) — opaque tokens in SQLite + HTTP-only cookies."""

from __future__ import annotations

import secrets
import sqlite3
import threading
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

_lock = threading.Lock()


@dataclass(frozen=True)
class SessionRecord:
    token: str
    user_id: int
    expires_at: datetime


def _connect(db_path: Path) -> sqlite3.Connection:
    db_path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(db_path), check_same_thread=False)
    conn.row_factory = sqlite3.Row
    return conn


def ensure_sessions_schema(db_path: Path) -> None:
    with _connect(db_path) as conn:
        conn.execute("PRAGMA journal_mode=WAL")
        conn.execute(
            """
            CREATE TABLE IF NOT EXISTS operator_sessions (
                token TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                revoked_at TEXT
            )
            """
        )
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_operator_sessions_user ON operator_sessions(user_id)"
        )
        conn.commit()


def _purge_expired(conn: sqlite3.Connection) -> None:
    now = datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    conn.execute("DELETE FROM operator_sessions WHERE expires_at < ?", (now,))


def create_session(db_path: Path, user_id: int, ttl_seconds: int) -> SessionRecord:
    """Insert a new random session token; returns token + expiry."""
    token = secrets.token_urlsafe(32)
    created = datetime.now(UTC).replace(microsecond=0)
    expires = created + timedelta(seconds=int(ttl_seconds))
    created_s = created.isoformat().replace("+00:00", "Z")
    expires_s = expires.isoformat().replace("+00:00", "Z")
    with _lock:
        ensure_sessions_schema(db_path)
        with _connect(db_path) as conn:
            _purge_expired(conn)
            conn.execute(
                """
                INSERT INTO operator_sessions (token, user_id, created_at, expires_at, revoked_at)
                VALUES (?, ?, ?, ?, NULL)
                """,
                (token, user_id, created_s, expires_s),
            )
            conn.commit()
    return SessionRecord(token=token, user_id=user_id, expires_at=expires)


def revoke_session(db_path: Path, token: str) -> bool:
    """Mark session revoked; returns True if a row was updated."""
    if not token.strip():
        return False
    now = datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    with _lock:
        ensure_sessions_schema(db_path)
        with _connect(db_path) as conn:
            cur = conn.execute(
                "UPDATE operator_sessions SET revoked_at = ? WHERE token = ? AND revoked_at IS NULL",
                (now, token),
            )
            conn.commit()
            return cur.rowcount > 0


def resolve_session_user_id(db_path: Path, token: str | None) -> int | None:
    """Return user_id if token is valid (not expired, not revoked)."""
    if not token or not token.strip():
        return None
    ensure_sessions_schema(db_path)
    with _connect(db_path) as conn:
        _purge_expired(conn)
        conn.commit()
        row = conn.execute(
            """
            SELECT user_id, expires_at, revoked_at FROM operator_sessions
            WHERE token = ?
            """,
            (token,),
        ).fetchone()
    if row is None:
        return None
    if row["revoked_at"] is not None:
        return None
    exp_s = str(row["expires_at"])
    exp_dt = datetime.fromisoformat(exp_s.replace("Z", "+00:00"))
    if exp_dt <= datetime.now(UTC):
        return None
    return int(row["user_id"])


def session_status(db_path: Path) -> dict[str, Any]:
    ensure_sessions_schema(db_path)
    with _connect(db_path) as conn:
        n = conn.execute(
            "SELECT COUNT(*) FROM operator_sessions WHERE revoked_at IS NULL"
        ).fetchone()
    return {"active_session_rows": int(n[0]) if n else 0}
