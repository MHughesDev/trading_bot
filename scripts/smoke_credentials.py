#!/usr/bin/env python3
"""
Smoke-test external APIs without placing orders or printing secrets.

- Coinbase: public REST candles (no auth). If NM_COINBASE_API_KEY is set, reports presence only.
- Alpaca paper: fetch_positions() (read-only) when NM_ALPACA_* are set.

Usage (from repo root, with .env or exported NM_* vars):

  pip install -e ".[alpaca]"
  python scripts/smoke_credentials.py
"""

from __future__ import annotations

import asyncio
import sys
from datetime import UTC, datetime, timedelta

import httpx

from app.config.settings import load_settings
from data_plane.ingest.coinbase_rest import CoinbaseRESTClient


async def _coinbase_public() -> tuple[bool, str]:
    """
    Try Advanced Trade public candles first. Some environments return 401 without JWT;
    then fall back to legacy Exchange public ticker (no auth) to prove reachability.
    """
    end = datetime.now(UTC)
    start = end - timedelta(hours=2)
    client = CoinbaseRESTClient()
    try:
        candles = await client.get_public_candles(
            "BTC-USD",
            start=start,
            end=end,
            granularity_seconds=3600,
        )
        if candles:
            return True, "Advanced Trade REST candles OK"
    except Exception as e:
        if "401" in str(e) or "403" in str(e):
            pass  # fall through to Exchange
        else:
            raise
    finally:
        await client.aclose()

    # Legacy Coinbase Exchange API — public, widely available
    url = "https://api.exchange.coinbase.com/products/BTC-USD/ticker"
    async with httpx.AsyncClient(timeout=30.0) as h:
        r = await h.get(url)
        r.raise_for_status()
        body = r.json()
    price = body.get("price")
    return bool(price), f"Exchange public ticker OK (last ~{price}) [Advanced Trade may require JWT]"


async def _alpaca_positions() -> tuple[bool, str]:
    settings = load_settings()
    if not settings.alpaca_api_key or not settings.alpaca_api_secret:
        return False, "skip (NM_ALPACA_API_KEY / NM_ALPACA_API_SECRET not set)"
    from execution.adapters.alpaca_paper import AlpacaPaperExecutionAdapter

    ad = AlpacaPaperExecutionAdapter(settings)
    positions = await ad.fetch_positions()
    return True, f"ok ({len(positions)} position row(s))"


async def main() -> int:
    settings = load_settings()
    print("NautilusMonster — credential / connectivity smoke test\n")

    # Coinbase connectivity (public path; keys not required for read in Exchange API)
    try:
        ok, detail = await _coinbase_public()
        print(f"Coinbase market data: {'OK' if ok else 'WARN'} — {detail}")
    except Exception as e:
        print(f"Coinbase market data: FAIL — {type(e).__name__}: {e}")
        return 1

    if settings.coinbase_api_key and settings.coinbase_api_secret:
        print("Coinbase API key/secret: present (private JWT order path not exercised here)")
    else:
        print("Coinbase API key/secret: not set (public data only; live orders need CDP signing in adapter)")

    # Alpaca paper
    try:
        ran, msg = await _alpaca_positions()
        if ran:
            print(f"Alpaca paper (fetch_positions): {msg}")
        else:
            print(f"Alpaca paper: {msg}")
    except Exception as e:
        print(f"Alpaca paper: FAIL — {type(e).__name__}: {e}")
        return 1

    print("\nDone. No orders were submitted.")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
