"""SQLite persistence for Coinbase tradable products (FB-AP-021)."""

from __future__ import annotations

import sqlite3
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

_META_KEY_SYNC = "last_sync_utc"
_META_KEY_ERROR = "last_error"


def _utc_now_iso() -> str:
    return datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def ensure_coinbase_universe_schema(db_path: Path) -> None:
    db_path.parent.mkdir(parents=True, exist_ok=True)
    con = sqlite3.connect(str(db_path))
    try:
        con.execute("PRAGMA foreign_keys=ON")
        con.executescript(
            """
            CREATE TABLE IF NOT EXISTS coinbase_universe_meta (
                key TEXT PRIMARY KEY,
                value TEXT
            );
            CREATE TABLE IF NOT EXISTS coinbase_tradable_products (
                product_id TEXT NOT NULL PRIMARY KEY,
                base_name TEXT,
                quote_name TEXT,
                product_type TEXT,
                trading_disabled INTEGER NOT NULL DEFAULT 0,
                is_disabled INTEGER NOT NULL DEFAULT 0,
                raw_json TEXT,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_coinbase_tradable_base ON coinbase_tradable_products(base_name);
            """
        )
        con.commit()
    finally:
        con.close()


def replace_coinbase_universe_rows(
    db_path: Path,
    rows: list[dict[str, Any]],
    *,
    sync_error: str | None = None,
) -> None:
    ensure_coinbase_universe_schema(db_path)
    now = _utc_now_iso()
    con = sqlite3.connect(str(db_path))
    try:
        con.execute("PRAGMA foreign_keys=ON")
        con.execute("DELETE FROM coinbase_tradable_products")
        for r in rows:
            con.execute(
                """
                INSERT INTO coinbase_tradable_products (
                    product_id, base_name, quote_name, product_type,
                    trading_disabled, is_disabled, raw_json, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    r["product_id"],
                    r.get("base_name") or "",
                    r.get("quote_name") or "",
                    r.get("product_type") or "",
                    1 if r.get("trading_disabled") else 0,
                    1 if r.get("is_disabled") else 0,
                    r.get("raw_json") or "{}",
                    now,
                ),
            )
        con.execute(
            "INSERT INTO coinbase_universe_meta(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            (_META_KEY_SYNC, now),
        )
        err_val = sync_error if sync_error else ""
        con.execute(
            "INSERT INTO coinbase_universe_meta(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            (_META_KEY_ERROR, err_val),
        )
        con.commit()
    finally:
        con.close()


def set_coinbase_universe_sync_error(db_path: Path, message: str | None) -> None:
    ensure_coinbase_universe_schema(db_path)
    con = sqlite3.connect(str(db_path))
    try:
        con.execute(
            "INSERT INTO coinbase_universe_meta(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            (_META_KEY_ERROR, message or ""),
        )
        con.commit()
    finally:
        con.close()


def list_coinbase_universe_rows(
    db_path: Path,
    *,
    limit: int = 100,
    offset: int = 0,
    query: str | None = None,
) -> tuple[list[dict[str, Any]], int]:
    ensure_coinbase_universe_schema(db_path)
    lim = max(1, min(int(limit), 10_000))
    off = max(0, int(offset))
    q = (query or "").strip().lower()
    con = sqlite3.connect(str(db_path))
    try:
        con.row_factory = sqlite3.Row
        if q:
            like = f"%{q}%"
            cur = con.execute(
                "SELECT COUNT(*) FROM coinbase_tradable_products WHERE "
                "LOWER(product_id) LIKE ? OR LOWER(COALESCE(base_name,'')) LIKE ?",
                (like, like),
            )
            total = int(cur.fetchone()[0])
            cur2 = con.execute(
                """
                SELECT product_id, base_name, quote_name, product_type, trading_disabled, is_disabled, updated_at
                FROM coinbase_tradable_products
                WHERE LOWER(product_id) LIKE ? OR LOWER(COALESCE(base_name,'')) LIKE ?
                ORDER BY product_id
                LIMIT ? OFFSET ?
                """,
                (like, like, lim, off),
            )
        else:
            cur = con.execute("SELECT COUNT(*) FROM coinbase_tradable_products")
            total = int(cur.fetchone()[0])
            cur2 = con.execute(
                """
                SELECT product_id, base_name, quote_name, product_type, trading_disabled, is_disabled, updated_at
                FROM coinbase_tradable_products
                ORDER BY product_id
                LIMIT ? OFFSET ?
                """,
                (lim, off),
            )
        rows = [dict(r) for r in cur2.fetchall()]
        for r in rows:
            r["trading_disabled"] = bool(r["trading_disabled"])
            r["is_disabled"] = bool(r["is_disabled"])
        return rows, total
    finally:
        con.close()


def coinbase_universe_status(db_path: Path) -> dict[str, Any]:
    ensure_coinbase_universe_schema(db_path)
    con = sqlite3.connect(str(db_path))
    try:
        n = int(con.execute("SELECT COUNT(*) FROM coinbase_tradable_products").fetchone()[0])
        last_sync = None
        last_err = None
        row = con.execute(
            "SELECT value FROM coinbase_universe_meta WHERE key = ?",
            (_META_KEY_SYNC,),
        ).fetchone()
        if row:
            last_sync = row[0]
        row = con.execute(
            "SELECT value FROM coinbase_universe_meta WHERE key = ?",
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
