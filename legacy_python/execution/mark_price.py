"""Mark price for portfolio rows: Kraken mid vs venue-only (FB-DASH-04-01)."""

from __future__ import annotations

import asyncio
from decimal import Decimal
from typing import Literal

from app.config.settings import AppSettings
from data_plane.ingest.kraken_rest import KrakenRESTClient
from data_plane.ingest.kraken_symbols import kraken_rest_pair

MarkPriceSource = Literal["kraken_mid", "venue_only"]


def effective_mark_price_source(settings: AppSettings) -> MarkPriceSource:
    """Paper defaults to Kraken mid; live defaults to venue-only."""
    if settings.execution_mode == "paper":
        return settings.portfolio_mark_price_source_paper
    return settings.portfolio_mark_price_source_live


async def fetch_kraken_mid_prices(
    symbols: list[str],
    *,
    client: KrakenRESTClient | None = None,
) -> dict[str, Decimal]:
    """Best bid/ask mid from Kraken public ``/Ticker`` (one REST call per distinct pair)."""
    own = client is None
    c = client or KrakenRESTClient()
    try:
        unique_pairs = list(dict.fromkeys(kraken_rest_pair(s) for s in symbols))
        if not unique_pairs:
            return {}
        mids = await asyncio.gather(*[c.ticker_mid(p) for p in unique_pairs])
        pair_to_mid = {p: m for p, m in zip(unique_pairs, mids, strict=True) if m is not None}
        out: dict[str, Decimal] = {}
        for sym in symbols:
            kp = kraken_rest_pair(sym)
            mid = pair_to_mid.get(kp)
            if mid is not None:
                out[sym] = Decimal(str(mid))
        return out
    finally:
        if own:
            await c.aclose()


def compute_unrealized_pnl(
    quantity: Decimal,
    avg_entry: Decimal | None,
    mark: Decimal | None,
) -> Decimal | None:
    """``qty * (mark - avg)`` when all operands are present."""
    if avg_entry is None or mark is None:
        return None
    return quantity * (mark - avg_entry)
