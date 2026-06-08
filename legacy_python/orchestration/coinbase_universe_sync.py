"""
Fetch Coinbase Advanced Trade **SPOT** products and persist to SQLite (FB-AP-021).

Uses :meth:`execution.coinbase_advanced_http.CoinbaseAdvancedHTTPClient.list_spot_products_paginated`
— execution venue metadata only; Kraken remains market data (**AGENTS.md**).
"""

from __future__ import annotations

import asyncio
import json
import logging
from typing import Any

from app.config.settings import AppSettings
from app.runtime.coinbase_universe_store import (
    replace_coinbase_universe_rows,
    set_coinbase_universe_sync_error,
)
from execution.coinbase_advanced_http import CoinbaseAdvancedHTTPClient

logger = logging.getLogger(__name__)


def _product_to_row(p: dict[str, Any]) -> dict[str, Any] | None:
    pid = str(p.get("product_id") or "").strip()
    if not pid:
        return None
    if p.get("trading_disabled") is True:
        return None
    if p.get("is_disabled") is True:
        return None
    try:
        raw_json = json.dumps(p, default=str)
    except TypeError:
        raw_json = "{}"
    return {
        "product_id": pid,
        "base_name": p.get("base_name") or p.get("base_currency_id") or None,
        "quote_name": p.get("quote_name") or p.get("quote_currency_id") or None,
        "product_type": str(p.get("product_type") or "SPOT"),
        "trading_disabled": bool(p.get("trading_disabled")),
        "is_disabled": bool(p.get("is_disabled")),
        "raw_json": raw_json,
    }


async def _fetch_all(settings: AppSettings) -> list[dict[str, Any]]:
    key = settings.coinbase_api_key.get_secret_value() if settings.coinbase_api_key else None
    sec = settings.coinbase_api_secret.get_secret_value() if settings.coinbase_api_secret else None
    if not key or not sec or not str(key).strip() or not str(sec).strip():
        raise RuntimeError("Coinbase API keys missing (NM_COINBASE_API_KEY / NM_COINBASE_API_SECRET)")
    client = CoinbaseAdvancedHTTPClient(key, sec)
    try:
        raw = await client.list_spot_products_paginated(limit=250, product_type="SPOT")
    finally:
        await client.aclose()
    rows: list[dict[str, Any]] = []
    for item in raw:
        row = _product_to_row(item)
        if row is not None:
            rows.append(row)
    rows.sort(key=lambda r: r["product_id"])
    return rows


def sync_coinbase_tradable_universe(settings: AppSettings) -> dict[str, Any]:
    """Replace SQLite snapshot with tradable SPOT products from Coinbase."""
    try:
        rows = asyncio.run(_fetch_all(settings))
        replace_coinbase_universe_rows(settings.coinbase_universe_db_path, rows, sync_error=None)
        logger.info("coinbase universe sync: stored %s spot product rows", len(rows))
        return {"ok": True, "count": len(rows), "error": None}
    except RuntimeError as e:
        err = str(e)
        logger.warning(err)
        set_coinbase_universe_sync_error(settings.coinbase_universe_db_path, err)
        return {"ok": False, "error": err, "count": 0}
    except Exception as e:
        err = str(e)
        logger.exception("coinbase universe sync failed")
        set_coinbase_universe_sync_error(settings.coinbase_universe_db_path, err)
        return {"ok": False, "error": err, "count": 0}
