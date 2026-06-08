#!/usr/bin/env python3
"""
Smoke-test external APIs without placing orders or printing secrets.

- Kraken: public REST OHLC (no auth). Live execution may still use Coinbase CDP keys separately.
- Alpaca paper: fetch_positions() (read-only) when NM_ALPACA_* are set.

Usage (from repo root, with .env or exported NM_* vars):

  pip install -e ".[alpaca]"
  python scripts/smoke_credentials.py
"""

from __future__ import annotations

import asyncio
import sys
from datetime import UTC, datetime, timedelta

from app.config.settings import load_settings
from data_plane.ingest.kraken_rest import KrakenRESTClient
from data_plane.ingest.kraken_symbols import kraken_rest_pair


async def _kraken_public() -> tuple[bool, str]:
    """GET /0/public/OHLC for BTC/USD (no auth)."""
    end = datetime.now(UTC)
    start = end - timedelta(hours=2)
    client = KrakenRESTClient()
    try:
        pair = kraken_rest_pair("BTC-USD")
        rows, _last = await client.ohlc(pair, interval_minutes=60, since=int(start.timestamp()))
        if rows:
            return True, f"Kraken REST OHLC OK ({len(rows)} row(s) for {pair})"
        return False, "no OHLC rows returned"
    finally:
        await client.aclose()


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
    print("Trading Bot — credential / connectivity smoke test\n")

    try:
        ok, detail = await _kraken_public()
        print(f"Kraken market data: {'OK' if ok else 'WARN'} — {detail}")
    except Exception as e:
        print(f"Kraken market data: FAIL — {type(e).__name__}: {e}")
        return 1

    if settings.coinbase_api_key and settings.coinbase_api_secret:
        print("Coinbase API key/secret: present (live Coinbase execution adapter only; not used for market data)")
    else:
        print("Coinbase API key/secret: not set (OK if not using live Coinbase execution)")

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
