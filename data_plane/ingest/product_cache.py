"""TTL cache of Coinbase product metadata for tick/min size validation."""

from __future__ import annotations

import time

from data_plane.ingest.coinbase_rest import CoinbaseProduct, CoinbaseRESTClient


class ProductMetadataCache:
    def __init__(self, client: CoinbaseRESTClient, ttl_seconds: float = 300.0) -> None:
        self._client = client
        self._ttl = ttl_seconds
        self._products: dict[str, CoinbaseProduct] = {}
        self._fetched_at: float = 0.0

    async def refresh_if_stale(self) -> None:
        now = time.monotonic()
        if now - self._fetched_at < self._ttl and self._products:
            return
        products = await self._client.list_products()
        self._products = {p.product_id: p for p in products}
        self._fetched_at = now

    def get(self, product_id: str) -> CoinbaseProduct | None:
        return self._products.get(product_id)

    def quote_increment(self, product_id: str) -> float | None:
        p = self.get(product_id)
        if not p:
            return None
        raw = p.raw
        for key in ("quote_increment", "quoteIncrement", "base_increment", "baseIncrement"):
            if key in raw and raw[key] is not None:
                try:
                    return float(raw[key])
                except (TypeError, ValueError):
                    continue
        return None

    def is_tradable(self, product_id: str) -> bool:
        p = self.get(product_id)
        if not p or not p.status:
            return True
        return p.status.lower() in ("online", "active", "trading")
