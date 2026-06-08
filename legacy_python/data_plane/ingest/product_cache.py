"""TTL cache of Kraken asset-pair metadata for tradable checks."""

from __future__ import annotations

import time

from data_plane.ingest.kraken_rest import KrakenRESTClient
from data_plane.ingest.kraken_symbols import kraken_pair_from_symbol


class ProductMetadataCache:
    def __init__(self, client: KrakenRESTClient, ttl_seconds: float = 300.0) -> None:
        self._client = client
        self._ttl = ttl_seconds
        self._wsname_tradable: dict[str, bool] = {}
        self._fetched_at: float = 0.0

    async def refresh_if_stale(self) -> None:
        now = time.monotonic()
        if now - self._fetched_at < self._ttl and self._wsname_tradable:
            return
        pairs = await self._client.list_asset_pairs()
        self._wsname_tradable = {}
        for _k, ap in pairs.items():
            name = ap.wsname
            if not name:
                continue
            st = (ap.raw.get("status") or "online").lower()
            self._wsname_tradable[name] = st in ("online", "reduce_only")
        self._fetched_at = now

    def is_tradable(self, symbol: str) -> bool:
        pair = kraken_pair_from_symbol(symbol)
        return self._wsname_tradable.get(pair, True)
