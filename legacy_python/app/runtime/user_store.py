"""SQLite-backed operator user accounts (FB-UX-001) — email + Argon2 password hash."""

from __future__ import annotations

import re
import sqlite3
import threading
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from argon2 import PasswordHasher
from argon2.exceptions import VerifyMismatchError

_EMAIL_RE = re.compile(r"^[^@\s]+@[^@\s]+\.[^@\s]+$")

_hasher = PasswordHasher()
_lock = threading.Lock()


class UserStoreError(Exception):
    """Base error for user store operations."""


class DuplicateEmailError(UserStoreError):
    """Email already registered."""

    def __init__(self, email: str) -> None:
        self.email = email
        super().__init__(f"email already registered: {email}")


class InvalidEmailError(UserStoreError):
    """Email failed basic validation."""

    def __str__(self) -> str:
        return self.args[0] if self.args else "invalid email"


class InvalidPasswordError(UserStoreError):
    """Password failed policy checks."""

    def __str__(self) -> str:
        return self.args[0] if self.args else "invalid password"


def normalize_email(raw: str) -> str:
    s = raw.strip().lower()
    if len(s) < 3 or len(s) > 254:
        raise InvalidEmailError("email length out of range")
    if not _EMAIL_RE.match(s):
        raise InvalidEmailError("email format invalid")
    return s


def validate_password(password: str) -> None:
    if len(password) < 8:
        raise InvalidPasswordError("password must be at least 8 characters")
    if len(password) > 1024:
        raise InvalidPasswordError("password too long")


def _connect(db_path: Path) -> sqlite3.Connection:
    db_path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(db_path), check_same_thread=False)
    conn.row_factory = sqlite3.Row
    return conn


def _init_schema(conn: sqlite3.Connection) -> None:
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            email TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            created_at TEXT NOT NULL
        )
        """
    )
    conn.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_users_email ON users(email)")
    conn.commit()


@dataclass(frozen=True)
class UserRecord:
    id: int
    email: str
    created_at: datetime


def ensure_schema(db_path: Path) -> None:
    with _connect(db_path) as conn:
        _init_schema(conn)


def count_users(db_path: Path) -> int:
    ensure_schema(db_path)
    with _connect(db_path) as conn:
        row = conn.execute("SELECT COUNT(*) FROM users").fetchone()
        return int(row[0]) if row else 0


def create_user(db_path: Path, email: str, password: str) -> UserRecord:
    """Insert a user with Argon2-hashed password. Thread-safe."""
    norm = normalize_email(email)
    validate_password(password)
    pwd_hash = _hasher.hash(password)
    created = datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    with _lock:
        ensure_schema(db_path)
        with _connect(db_path) as conn:
            try:
                cur = conn.execute(
                    "INSERT INTO users (email, password_hash, created_at) VALUES (?, ?, ?)",
                    (norm, pwd_hash, created),
                )
                conn.commit()
                uid = int(cur.lastrowid)
            except sqlite3.IntegrityError as e:
                raise DuplicateEmailError(norm) from e
    return UserRecord(id=uid, email=norm, created_at=datetime.fromisoformat(created.replace("Z", "+00:00")))


def verify_password(db_path: Path, email: str, password: str) -> UserRecord | None:
    """Return the user if password matches; None if unknown email or bad password."""
    try:
        norm = normalize_email(email)
    except InvalidEmailError:
        return None
    ensure_schema(db_path)
    with _connect(db_path) as conn:
        row = conn.execute(
            "SELECT id, email, password_hash, created_at FROM users WHERE email = ?",
            (norm,),
        ).fetchone()
    if row is None:
        return None
    try:
        _hasher.verify(row["password_hash"], password)
    except VerifyMismatchError:
        return None
    created = str(row["created_at"])
    dt = datetime.fromisoformat(created.replace("Z", "+00:00"))
    return UserRecord(id=int(row["id"]), email=str(row["email"]), created_at=dt)


def get_user_by_id(db_path: Path, user_id: int) -> UserRecord | None:
    """Load user by primary key."""
    ensure_schema(db_path)
    with _connect(db_path) as conn:
        row = conn.execute(
            "SELECT id, email, created_at FROM users WHERE id = ?",
            (user_id,),
        ).fetchone()
    if row is None:
        return None
    created = str(row["created_at"])
    dt = datetime.fromisoformat(created.replace("Z", "+00:00"))
    return UserRecord(id=int(row["id"]), email=str(row["email"]), created_at=dt)


def user_store_status(db_path: Path) -> dict[str, Any]:
    """Payload fragment for GET /status."""
    n = count_users(db_path)
    return {
        "db_path": str(db_path.resolve()),
        "user_count": n,
    }
