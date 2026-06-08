"""SQLite persistence for Alpaca tradable crypto universe (FB-AP-020)."""

from __future__ import annotations

import sqlite3
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

_META_KEY_SYNC = "last_sync_utc"
_META_KEY_ERROR = "last_error"


def _utc_now_iso() -> str:
    return datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def ensure_alpaca_universe_schema(db_path: Path) -> None:
    db_path.parent.mkdir(parents=True, exist_ok=True)
    con = sqlite3.connect(str(db_path))
    try:
        con.execute("PRAGMA foreign_keys=ON")
        con.executescript(
            """
            CREATE TABLE IF NOT EXISTS alpaca_universe_meta (
                key TEXT PRIMARY KEY,
                value TEXT
            );
            CREATE TABLE IF NOT EXISTS alpaca_tradable_crypto (
                canonical_symbol TEXT NOT NULL PRIMARY KEY,
                alpaca_symbol TEXT NOT NULL,
                name TEXT,
                asset_class TEXT,
                exchange TEXT,
                tradable INTEGER NOT NULL DEFAULT 0,
                raw_json TEXT,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_alpaca_tradable_name ON alpaca_tradable_crypto(name);
            """
        )
        con.commit()
    finally:
        con.close()


def replace_alpaca_universe_rows(
    db_path: Path,
    rows: list[dict[str, Any]],
    *,
    sync_error: str | None = None,
) -> None:
    """Full refresh: delete all rows, insert new snapshot; update meta."""
    ensure_alpaca_universe_schema(db_path)
    now = _utc_now_iso()
    con = sqlite3.connect(str(db_path))
    try:
        con.execute("PRAGMA foreign_keys=ON")
        con.execute("DELETE FROM alpaca_tradable_crypto")
        for r in rows:
            con.execute(
                """
                INSERT INTO alpaca_tradable_crypto (
                    canonical_symbol, alpaca_symbol, name, asset_class, exchange, tradable, raw_json, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    r["canonical_symbol"],
                    r["alpaca_symbol"],
                    r.get("name") or "",
                    r.get("asset_class") or "",
                    r.get("exchange") or "",
                    1 if r.get("tradable") else 0,
                    r.get("raw_json") or "{}",
                    now,
                ),
            )
        con.execute(
            "INSERT INTO alpaca_universe_meta(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            (_META_KEY_SYNC, now),
        )
        err_val = sync_error if sync_error else ""
        con.execute(
            "INSERT INTO alpaca_universe_meta(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            (_META_KEY_ERROR, err_val),
        )
        con.commit()
    finally:
        con.close()


def list_alpaca_universe_rows(
    db_path: Path,
    *,
    limit: int = 100,
    offset: int = 0,
    query: str | None = None,
) -> tuple[list[dict[str, Any]], int]:
    """Return (rows, total_count) for tradable crypto rows."""
    ensure_alpaca_universe_schema(db_path)
    lim = max(1, min(int(limit), 10_000))
    off = max(0, int(offset))
    q = (query or "").strip().lower()
    con = sqlite3.connect(str(db_path))
    try:
        con.row_factory = sqlite3.Row
        if q:
            like = f"%{q}%"
            cur = con.execute(
                "SELECT COUNT(*) FROM alpaca_tradable_crypto WHERE "
                "LOWER(canonical_symbol) LIKE ? OR LOWER(alpaca_symbol) LIKE ? OR LOWER(COALESCE(name,'')) LIKE ?",
                (like, like, like),
            )
            total = int(cur.fetchone()[0])
            cur2 = con.execute(
                """
                SELECT canonical_symbol, alpaca_symbol, name, asset_class, exchange, tradable, updated_at
                FROM alpaca_tradable_crypto
                WHERE LOWER(canonical_symbol) LIKE ? OR LOWER(alpaca_symbol) LIKE ? OR LOWER(COALESCE(name,'')) LIKE ?
                ORDER BY canonical_symbol
                LIMIT ? OFFSET ?
                """,
                (like, like, like, lim, off),
            )
        else:
            cur = con.execute("SELECT COUNT(*) FROM alpaca_tradable_crypto")
            total = int(cur.fetchone()[0])
            cur2 = con.execute(
                """
                SELECT canonical_symbol, alpaca_symbol, name, asset_class, exchange, tradable, updated_at
                FROM alpaca_tradable_crypto
                ORDER BY canonical_symbol
                LIMIT ? OFFSET ?
                """,
                (lim, off),
            )
        rows = [dict(r) for r in cur2.fetchall()]
        for r in rows:
            r["tradable"] = bool(r["tradable"])
        return rows, total
    finally:
        con.close()


def set_alpaca_universe_sync_error(db_path: Path, message: str | None) -> None:
    """Record last sync error without clearing rows."""
    ensure_alpaca_universe_schema(db_path)
    con = sqlite3.connect(str(db_path))
    try:
        con.execute(
            "INSERT INTO alpaca_universe_meta(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            (_META_KEY_ERROR, message or ""),
        )
        con.commit()
    finally:
        con.close()


def alpaca_universe_status(db_path: Path) -> dict[str, Any]:
    ensure_alpaca_universe_schema(db_path)
    con = sqlite3.connect(str(db_path))
    try:
        n = int(con.execute("SELECT COUNT(*) FROM alpaca_tradable_crypto").fetchone()[0])
        last_sync = None
        last_err = None
        row = con.execute(
            "SELECT value FROM alpaca_universe_meta WHERE key = ?",
            (_META_KEY_SYNC,),
        ).fetchone()
        if row:
            last_sync = row[0]
        row = con.execute(
            "SELECT value FROM alpaca_universe_meta WHERE key = ?",
            (_META_KEY_ERROR,),
        ).fetchone()
        if row and row[0]:
            last_err = row[0]
        return {
            "db_path": str(db_path),
            "row_count": n,
            "last_sync_utc": last_sync,
            "last_error": last_err,
        }
    finally:
        con.close()
