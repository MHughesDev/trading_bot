"""Per-user Alpaca / Coinbase API credentials — encrypted at rest (FB-UX-006)."""

from __future__ import annotations

import base64
import hashlib
import sqlite3
import threading
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from cryptography.fernet import Fernet, InvalidToken

_lock = threading.Lock()


def _fernet(master_secret: str) -> Fernet:
    key = base64.urlsafe_b64encode(hashlib.sha256(master_secret.encode("utf-8")).digest())
    return Fernet(key)


def _connect(db_path: Path) -> sqlite3.Connection:
    conn = sqlite3.connect(str(db_path), check_same_thread=False)
    conn.row_factory = sqlite3.Row
    return conn


def ensure_venue_schema(db_path: Path) -> None:
    with _connect(db_path) as conn:
        conn.execute("PRAGMA journal_mode=WAL")
        conn.execute("PRAGMA foreign_keys=ON")
        conn.execute(
            """
            CREATE TABLE IF NOT EXISTS user_venue_credentials (
                user_id INTEGER PRIMARY KEY,
                alpaca_key_enc BLOB,
                alpaca_secret_enc BLOB,
                coinbase_key_enc BLOB,
                coinbase_secret_enc BLOB,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id)
            )
            """
        )
        conn.commit()


def _enc(f: Fernet, plain: str | None) -> bytes | None:
    if plain is None or not str(plain).strip():
        return None
    return f.encrypt(str(plain).encode("utf-8"))


def _dec(f: Fernet, blob: bytes | None) -> str | None:
    if blob is None:
        return None
    try:
        return f.decrypt(blob).decode("utf-8")
    except InvalidToken:
        return None


def mask_secret(s: str | None, *, last: int = 4) -> str | None:
    if not s:
        return None
    t = str(s).strip()
    if len(t) <= last:
        return "****"
    return "****" + t[-last:]


def save_credentials(
    db_path: Path,
    master_secret: str,
    user_id: int,
    *,
    alpaca_api_key: str | None = None,
    alpaca_api_secret: str | None = None,
    coinbase_api_key: str | None = None,
    coinbase_api_secret: str | None = None,
    clear_alpaca: bool = False,
    clear_coinbase: bool = False,
) -> None:
    """Upsert encrypted columns; None means leave unchanged unless clear_* is True."""
    f = _fernet(master_secret)
    with _lock:
        ensure_venue_schema(db_path)
        with _connect(db_path) as conn:
            row = conn.execute(
                "SELECT alpaca_key_enc, alpaca_secret_enc, coinbase_key_enc, coinbase_secret_enc "
                "FROM user_venue_credentials WHERE user_id = ?",
                (user_id,),
            ).fetchone()
            ak = row["alpaca_key_enc"] if row else None
            asec = row["alpaca_secret_enc"] if row else None
            ck = row["coinbase_key_enc"] if row else None
            csec = row["coinbase_secret_enc"] if row else None

            if clear_alpaca:
                ak, asec = None, None
            else:
                if alpaca_api_key is not None:
                    ak = _enc(f, alpaca_api_key)
                if alpaca_api_secret is not None:
                    asec = _enc(f, alpaca_api_secret)

            if clear_coinbase:
                ck, csec = None, None
            else:
                if coinbase_api_key is not None:
                    ck = _enc(f, coinbase_api_key)
                if coinbase_api_secret is not None:
                    csec = _enc(f, coinbase_api_secret)

            now = datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")
            conn.execute(
                """
                INSERT INTO user_venue_credentials (
                    user_id, alpaca_key_enc, alpaca_secret_enc, coinbase_key_enc, coinbase_secret_enc, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?)
                ON CONFLICT(user_id) DO UPDATE SET
                    alpaca_key_enc = excluded.alpaca_key_enc,
                    alpaca_secret_enc = excluded.alpaca_secret_enc,
                    coinbase_key_enc = excluded.coinbase_key_enc,
                    coinbase_secret_enc = excluded.coinbase_secret_enc,
                    updated_at = excluded.updated_at
                """,
                (user_id, ak, asec, ck, csec, now),
            )
            conn.commit()


def load_masked(db_path: Path, master_secret: str, user_id: int) -> dict[str, Any]:
    """Return API-safe dict (masked secrets, booleans for presence)."""
    ensure_venue_schema(db_path)
    f = _fernet(master_secret)
    with _connect(db_path) as conn:
        row = conn.execute(
            "SELECT alpaca_key_enc, alpaca_secret_enc, coinbase_key_enc, coinbase_secret_enc, updated_at "
            "FROM user_venue_credentials WHERE user_id = ?",
            (user_id,),
        ).fetchone()
    if row is None:
        return {
            "alpaca_key_set": False,
            "alpaca_secret_set": False,
            "coinbase_key_set": False,
            "coinbase_secret_set": False,
            "alpaca_key_masked": None,
            "alpaca_secret_masked": None,
            "coinbase_key_masked": None,
            "coinbase_secret_masked": None,
            "updated_at": None,
        }
    ak = _dec(f, row["alpaca_key_enc"])
    asec = _dec(f, row["alpaca_secret_enc"])
    ck = _dec(f, row["coinbase_key_enc"])
    csec = _dec(f, row["coinbase_secret_enc"])
    return {
        "alpaca_key_set": bool(ak),
        "alpaca_secret_set": bool(asec),
        "coinbase_key_set": bool(ck),
        "coinbase_secret_set": bool(csec),
        "alpaca_key_masked": mask_secret(ak),
        "alpaca_secret_masked": mask_secret(asec),
        "coinbase_key_masked": mask_secret(ck),
        "coinbase_secret_masked": mask_secret(csec),
        "updated_at": str(row["updated_at"]) if row["updated_at"] else None,
    }
