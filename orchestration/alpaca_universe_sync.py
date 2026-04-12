"""
Fetch Alpaca **tradable crypto** assets and persist to SQLite (FB-AP-020).

Uses ``TradingClient.get_all_assets`` with ``AssetClass.CRYPTO`` — execution venue only;
Kraken remains the market-data path (**AGENTS.md**).
"""

from __future__ import annotations

import json
import logging
from typing import Any

from app.config.settings import AppSettings
from app.runtime.alpaca_universe_store import replace_alpaca_universe_rows, set_alpaca_universe_sync_error
from execution.alpaca_util import from_alpaca_crypto_symbol

logger = logging.getLogger(__name__)


def _asset_to_row(a: Any) -> dict[str, Any] | None:
    sym = str(getattr(a, "symbol", "") or "").strip().upper()
    if not sym:
        return None
    tradable = bool(getattr(a, "tradable", False))
    if not tradable:
        return None
    try:
        canon = from_alpaca_crypto_symbol(sym)
    except Exception:
        canon = sym
    name = str(getattr(a, "name", "") or "")
    ac = str(getattr(a, "asset_class", "") or getattr(a, "class_", "") or "")
    ex = str(getattr(a, "exchange", "") or "")
    raw: dict[str, Any] = {
        "symbol": sym,
        "name": name,
        "asset_class": ac,
        "exchange": ex,
        "tradable": tradable,
    }
    aid = getattr(a, "id", None)
    if aid is not None:
        raw["id"] = str(aid)
    try:
        raw_json = json.dumps(raw, default=str)
    except TypeError:
        raw_json = "{}"
    return {
        "canonical_symbol": canon,
        "alpaca_symbol": sym,
        "name": name or None,
        "asset_class": ac or None,
        "exchange": ex or None,
        "tradable": True,
        "raw_json": raw_json,
    }


def sync_alpaca_tradable_universe(settings: AppSettings) -> dict[str, Any]:
    """
    Pull tradable crypto assets from Alpaca paper API and replace local SQLite snapshot.

    Requires ``NM_ALPACA_API_KEY`` / ``NM_ALPACA_API_SECRET`` and ``alpaca-py``.
    """
    try:
        from alpaca.trading.client import TradingClient
        from alpaca.trading.enums import AssetClass, AssetStatus
        from alpaca.trading.requests import GetAssetsRequest
    except ImportError as e:
        err = f"alpaca-py not installed: {e}"
        logger.warning(err)
        set_alpaca_universe_sync_error(settings.alpaca_universe_db_path, err)
        return {"ok": False, "error": err, "count": 0}

    key = settings.alpaca_api_key.get_secret_value() if settings.alpaca_api_key else None
    sec = settings.alpaca_api_secret.get_secret_value() if settings.alpaca_api_secret else None
    if not key or not sec or not str(key).strip() or not str(sec).strip():
        err = "Alpaca API keys missing (NM_ALPACA_API_KEY / NM_ALPACA_API_SECRET)"
        logger.warning(err)
        set_alpaca_universe_sync_error(settings.alpaca_universe_db_path, err)
        return {"ok": False, "error": err, "count": 0}

    def _run() -> list[dict[str, Any]]:
        client = TradingClient(key, sec, paper=True)
        req = GetAssetsRequest(asset_class=AssetClass.CRYPTO, status=AssetStatus.ACTIVE)
        assets = client.get_all_assets(req)
        rows: list[dict[str, Any]] = []
        for a in assets or []:
            row = _asset_to_row(a)
            if row is not None:
                rows.append(row)
        rows.sort(key=lambda r: r["canonical_symbol"])
        return rows

    try:
        rows = _run()
        replace_alpaca_universe_rows(settings.alpaca_universe_db_path, rows, sync_error=None)
        logger.info("alpaca universe sync: stored %s tradable crypto rows", len(rows))
        return {"ok": True, "count": len(rows), "error": None}
    except Exception as e:
        err = str(e)
        logger.exception("alpaca universe sync failed")
        set_alpaca_universe_sync_error(settings.alpaca_universe_db_path, err)
        return {"ok": False, "error": err, "count": 0}
