"""
Cross-venue **platform-supported** symbol set (FB-AP-022).

Rule version **1**: ``intersection`` — a canonical pair (e.g. ``BTC-USD``) is *platform-supported*
only if it appears in **both** the Alpaca tradable-crypto snapshot **and** the Coinbase SPOT snapshot.

This set is for **search and eligibility hints only**; Kraken pair mapping and market data are unchanged
(**AGENTS.md**).

Optional ``union`` mode lists symbols tradable on **either** venue (debug / operator visibility).
"""

from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Any, Literal

RULE_VERSION = 1

PlatformSupportedMode = Literal["intersection", "union"]


def _attach_coinbase(con: sqlite3.Connection, coinbase_db_path: Path) -> None:
    con.execute("ATTACH ? AS cb", (str(coinbase_db_path),))


def _detach_coinbase(con: sqlite3.Connection) -> None:
    con.execute("DETACH DATABASE cb")


def platform_supported_count(
    alpaca_db_path: Path,
    coinbase_db_path: Path,
    *,
    mode: PlatformSupportedMode = "intersection",
) -> int:
    """Row count for the configured cross-reference (both DBs must exist or be creatable)."""
    con = sqlite3.connect(str(alpaca_db_path))
    try:
        _attach_coinbase(con, coinbase_db_path)
        if mode == "intersection":
            cur = con.execute(
                """
                SELECT COUNT(*) FROM (
                    SELECT canonical_symbol FROM alpaca_tradable_crypto
                    INTERSECT
                    SELECT product_id FROM cb.coinbase_tradable_products
                )
                """
            )
        else:
            cur = con.execute(
                """
                SELECT COUNT(*) FROM (
                    SELECT canonical_symbol AS s FROM alpaca_tradable_crypto
                    UNION
                    SELECT product_id AS s FROM cb.coinbase_tradable_products
                )
                """
            )
        return int(cur.fetchone()[0])
    finally:
        try:
            _detach_coinbase(con)
        except sqlite3.Error:
            pass
        con.close()


def list_platform_supported_symbols(
    alpaca_db_path: Path,
    coinbase_db_path: Path,
    *,
    mode: PlatformSupportedMode = "intersection",
    limit: int = 200,
    offset: int = 0,
    query: str | None = None,
) -> tuple[list[str], int]:
    """
    Return (symbols, total) for the platform-supported set.

    ``query`` filters case-insensitively on symbol and (when present) Alpaca name / Coinbase base name.
    """
    lim = max(1, min(int(limit), 10_000))
    off = max(0, int(offset))
    q = (query or "").strip().lower()

    con = sqlite3.connect(str(alpaca_db_path))
    try:
        _attach_coinbase(con, coinbase_db_path)
        if mode == "intersection":
            if q:
                like = f"%{q}%"
                cur_tot = con.execute(
                    """
                    SELECT COUNT(*) FROM alpaca_tradable_crypto a
                    INNER JOIN cb.coinbase_tradable_products c ON a.canonical_symbol = c.product_id
                    WHERE LOWER(a.canonical_symbol) LIKE ? OR LOWER(COALESCE(a.name,'')) LIKE ?
                        OR LOWER(COALESCE(c.base_name,'')) LIKE ?
                    """,
                    (like, like, like),
                )
                total = int(cur_tot.fetchone()[0])
                cur_rows = con.execute(
                    """
                    SELECT a.canonical_symbol FROM alpaca_tradable_crypto a
                    INNER JOIN cb.coinbase_tradable_products c ON a.canonical_symbol = c.product_id
                    WHERE LOWER(a.canonical_symbol) LIKE ? OR LOWER(COALESCE(a.name,'')) LIKE ?
                        OR LOWER(COALESCE(c.base_name,'')) LIKE ?
                    ORDER BY a.canonical_symbol
                    LIMIT ? OFFSET ?
                    """,
                    (like, like, like, lim, off),
                )
            else:
                cur_tot = con.execute(
                    """
                    SELECT COUNT(*) FROM (
                        SELECT canonical_symbol FROM alpaca_tradable_crypto
                        INTERSECT
                        SELECT product_id FROM cb.coinbase_tradable_products
                    )
                    """
                )
                total = int(cur_tot.fetchone()[0])
                cur_rows = con.execute(
                    """
                    SELECT canonical_symbol FROM (
                        SELECT canonical_symbol FROM alpaca_tradable_crypto
                        INTERSECT
                        SELECT product_id FROM cb.coinbase_tradable_products
                    )
                    ORDER BY canonical_symbol
                    LIMIT ? OFFSET ?
                    """,
                    (lim, off),
                )
        else:
            # union
            if q:
                like = f"%{q}%"
                cur_tot = con.execute(
                    """
                    SELECT COUNT(*) FROM (
                        SELECT canonical_symbol AS s FROM alpaca_tradable_crypto
                        WHERE LOWER(canonical_symbol) LIKE ? OR LOWER(COALESCE(name,'')) LIKE ?
                        UNION
                        SELECT product_id AS s FROM cb.coinbase_tradable_products
                        WHERE LOWER(product_id) LIKE ? OR LOWER(COALESCE(base_name,'')) LIKE ?
                    )
                    """,
                    (like, like, like, like),
                )
                total = int(cur_tot.fetchone()[0])
                cur_rows = con.execute(
                    """
                    SELECT s FROM (
                        SELECT canonical_symbol AS s FROM alpaca_tradable_crypto
                        WHERE LOWER(canonical_symbol) LIKE ? OR LOWER(COALESCE(name,'')) LIKE ?
                        UNION
                        SELECT product_id AS s FROM cb.coinbase_tradable_products
                        WHERE LOWER(product_id) LIKE ? OR LOWER(COALESCE(base_name,'')) LIKE ?
                    )
                    ORDER BY s
                    LIMIT ? OFFSET ?
                    """,
                    (like, like, like, like, lim, off),
                )
            else:
                cur_tot = con.execute(
                    """
                    SELECT COUNT(*) FROM (
                        SELECT canonical_symbol AS s FROM alpaca_tradable_crypto
                        UNION
                        SELECT product_id AS s FROM cb.coinbase_tradable_products
                    )
                    """
                )
                total = int(cur_tot.fetchone()[0])
                cur_rows = con.execute(
                    """
                    SELECT s FROM (
                        SELECT canonical_symbol AS s FROM alpaca_tradable_crypto
                        UNION
                        SELECT product_id AS s FROM cb.coinbase_tradable_products
                    )
                    ORDER BY s
                    LIMIT ? OFFSET ?
                    """,
                    (lim, off),
                )

        rows = [str(r[0]) for r in cur_rows.fetchall()]
        return rows, total
    finally:
        try:
            _detach_coinbase(con)
        except sqlite3.Error:
            pass
        con.close()


def list_platform_supported_search_rows(
    alpaca_db_path: Path,
    coinbase_db_path: Path,
    *,
    mode: PlatformSupportedMode = "intersection",
    limit: int = 200,
    offset: int = 0,
    query: str | None = None,
) -> tuple[list[dict[str, Any]], int]:
    """
    Paginated **metadata** rows for universe search (FB-AP-023): symbols from the FB-AP-022 set
    with optional Alpaca / Coinbase fields — **no** OHLC or bar series.
    """
    lim = max(1, min(int(limit), 10_000))
    off = max(0, int(offset))
    q = (query or "").strip().lower()

    con = sqlite3.connect(str(alpaca_db_path))
    try:
        _attach_coinbase(con, coinbase_db_path)
        if mode == "intersection":
            if q:
                like = f"%{q}%"
                cur_tot = con.execute(
                    """
                    SELECT COUNT(*) FROM alpaca_tradable_crypto a
                    INNER JOIN cb.coinbase_tradable_products c ON a.canonical_symbol = c.product_id
                    WHERE LOWER(a.canonical_symbol) LIKE ? OR LOWER(COALESCE(a.name,'')) LIKE ?
                        OR LOWER(COALESCE(c.base_name,'')) LIKE ?
                    """,
                    (like, like, like),
                )
                total = int(cur_tot.fetchone()[0])
                cur_rows = con.execute(
                    """
                    SELECT a.canonical_symbol, a.alpaca_symbol, a.name,
                           c.product_id, c.base_name, c.quote_name
                    FROM alpaca_tradable_crypto a
                    INNER JOIN cb.coinbase_tradable_products c ON a.canonical_symbol = c.product_id
                    WHERE LOWER(a.canonical_symbol) LIKE ? OR LOWER(COALESCE(a.name,'')) LIKE ?
                        OR LOWER(COALESCE(c.base_name,'')) LIKE ?
                    ORDER BY a.canonical_symbol
                    LIMIT ? OFFSET ?
                    """,
                    (like, like, like, lim, off),
                )
            else:
                cur_tot = con.execute(
                    """
                    SELECT COUNT(*) FROM (
                        SELECT canonical_symbol FROM alpaca_tradable_crypto
                        INTERSECT
                        SELECT product_id FROM cb.coinbase_tradable_products
                    )
                    """
                )
                total = int(cur_tot.fetchone()[0])
                cur_rows = con.execute(
                    """
                    SELECT a.canonical_symbol, a.alpaca_symbol, a.name,
                           c.product_id, c.base_name, c.quote_name
                    FROM alpaca_tradable_crypto a
                    INNER JOIN cb.coinbase_tradable_products c ON a.canonical_symbol = c.product_id
                    ORDER BY a.canonical_symbol
                    LIMIT ? OFFSET ?
                    """,
                    (lim, off),
                )
        else:
            if q:
                like = f"%{q}%"
                cur_tot = con.execute(
                    """
                    SELECT COUNT(*) FROM (
                        SELECT canonical_symbol AS s FROM alpaca_tradable_crypto
                        UNION
                        SELECT product_id AS s FROM cb.coinbase_tradable_products
                    ) sym
                    LEFT JOIN alpaca_tradable_crypto a ON a.canonical_symbol = sym.s
                    LEFT JOIN cb.coinbase_tradable_products c ON c.product_id = sym.s
                    WHERE LOWER(sym.s) LIKE ? OR LOWER(COALESCE(a.name,'')) LIKE ?
                        OR LOWER(COALESCE(c.base_name,'')) LIKE ?
                    """,
                    (like, like, like),
                )
                total = int(cur_tot.fetchone()[0])
                cur_rows = con.execute(
                    """
                    SELECT sym.s, a.alpaca_symbol, a.name,
                           c.product_id, c.base_name, c.quote_name
                    FROM (
                        SELECT canonical_symbol AS s FROM alpaca_tradable_crypto
                        UNION
                        SELECT product_id AS s FROM cb.coinbase_tradable_products
                    ) sym
                    LEFT JOIN alpaca_tradable_crypto a ON a.canonical_symbol = sym.s
                    LEFT JOIN cb.coinbase_tradable_products c ON c.product_id = sym.s
                    WHERE LOWER(sym.s) LIKE ? OR LOWER(COALESCE(a.name,'')) LIKE ?
                        OR LOWER(COALESCE(c.base_name,'')) LIKE ?
                    ORDER BY sym.s
                    LIMIT ? OFFSET ?
                    """,
                    (like, like, like, lim, off),
                )
            else:
                cur_tot = con.execute(
                    """
                    SELECT COUNT(*) FROM (
                        SELECT canonical_symbol AS s FROM alpaca_tradable_crypto
                        UNION
                        SELECT product_id AS s FROM cb.coinbase_tradable_products
                    )
                    """
                )
                total = int(cur_tot.fetchone()[0])
                cur_rows = con.execute(
                    """
                    SELECT sym.s, a.alpaca_symbol, a.name,
                           c.product_id, c.base_name, c.quote_name
                    FROM (
                        SELECT canonical_symbol AS s FROM alpaca_tradable_crypto
                        UNION
                        SELECT product_id AS s FROM cb.coinbase_tradable_products
                    ) sym
                    LEFT JOIN alpaca_tradable_crypto a ON a.canonical_symbol = sym.s
                    LEFT JOIN cb.coinbase_tradable_products c ON c.product_id = sym.s
                    ORDER BY sym.s
                    LIMIT ? OFFSET ?
                    """,
                    (lim, off),
                )

        out: list[dict[str, Any]] = []
        for row in cur_rows.fetchall():
            sym_s, alp_sym, alp_name, cb_pid, cb_base, cb_quote = row
            canon = str(sym_s)
            has_a = alp_sym is not None
            has_c = cb_pid is not None
            out.append(
                {
                    "canonical_symbol": canon,
                    "paper_tradable": has_a,
                    "live_tradable": has_c,
                    "alpaca_symbol": None if alp_sym is None else str(alp_sym),
                    "alpaca_name": None if alp_name is None or str(alp_name) == "" else str(alp_name),
                    "coinbase_product_id": None if cb_pid is None else str(cb_pid),
                    "coinbase_base_name": None if cb_base is None or str(cb_base) == "" else str(cb_base),
                    "coinbase_quote_name": None if cb_quote is None or str(cb_quote) == "" else str(cb_quote),
                }
            )
        return out, total
    finally:
        try:
            _detach_coinbase(con)
        except sqlite3.Error:
            pass
        con.close()


def universe_search_payload(
    settings: Any,
    *,
    limit: int = 200,
    offset: int = 0,
    query: str | None = None,
) -> dict[str, Any]:
    """JSON for ``GET /universe/search`` (FB-AP-023)."""
    from app.config.settings import AppSettings

    s = settings if isinstance(settings, AppSettings) else settings
    mode = s.platform_supported_universe_mode
    if mode not in ("intersection", "union"):
        mode = "intersection"
    rows, total = list_platform_supported_search_rows(
        s.alpaca_universe_db_path,
        s.coinbase_universe_db_path,
        mode=mode,  # type: ignore[arg-type]
        limit=limit,
        offset=offset,
        query=query,
    )
    return {
        "ok": True,
        "rule_version": RULE_VERSION,
        "mode": mode,
        "total": total,
        "limit": max(1, min(int(limit), 10_000)),
        "offset": max(0, int(offset)),
        "query": query,
        "rows": rows,
    }


def platform_supported_payload(
    settings: Any,
    *,
    limit: int = 200,
    offset: int = 0,
    query: str | None = None,
) -> dict[str, Any]:
    """JSON-serializable summary for ``GET /status`` and ``GET /universe/platform-supported``."""
    from app.config.settings import AppSettings

    s = settings if isinstance(settings, AppSettings) else settings
    mode = s.platform_supported_universe_mode
    if mode not in ("intersection", "union"):
        mode = "intersection"
    symbols, total = list_platform_supported_symbols(
        s.alpaca_universe_db_path,
        s.coinbase_universe_db_path,
        mode=mode,  # type: ignore[arg-type]
        limit=limit,
        offset=offset,
        query=query,
    )
    return {
        "ok": True,
        "rule_version": RULE_VERSION,
        "mode": mode,
        "definition": (
            "intersection: canonical_symbol in both Alpaca tradable crypto and Coinbase SPOT snapshots"
            if mode == "intersection"
            else "union: symbol appears in either snapshot (either-venue tradability)"
        ),
        "total": total,
        "limit": max(1, min(int(limit), 10_000)),
        "offset": max(0, int(offset)),
        "query": query,
        "symbols": symbols,
    }


def platform_supported_status_summary(settings: Any) -> dict[str, Any]:
    """Compact block for ``GET /status`` (counts only; no symbol list)."""
    from app.config.settings import AppSettings

    s = settings if isinstance(settings, AppSettings) else settings
    mode = s.platform_supported_universe_mode
    if mode not in ("intersection", "union"):
        mode = "intersection"
    n = platform_supported_count(
        s.alpaca_universe_db_path,
        s.coinbase_universe_db_path,
        mode=mode,  # type: ignore[arg-type]
    )
    return {
        "rule_version": RULE_VERSION,
        "mode": mode,
        "symbol_count": n,
    }
