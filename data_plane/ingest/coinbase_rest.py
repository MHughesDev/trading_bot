"""
Coinbase Advanced Trade REST client (market data ONLY).

Public endpoints do not require authentication. Signed requests for private trading use CDP keys.
https://docs.cdp.coinbase.com/advanced-trade/docs/rest-api
"""

from __future__ import annotations

import logging
import random
from datetime import UTC, datetime
from typing import Any

import httpx
from pydantic import BaseModel, Field
from pydantic_settings import BaseSettings, SettingsConfigDict

logger = logging.getLogger(__name__)

DEFAULT_BASE = "https://api.coinbase.com/api/v3/brokerage"


class CoinbaseRESTSettings(BaseSettings):
    model_config = SettingsConfigDict(env_prefix="COINBASE_REST_", env_file=".env", extra="ignore")

    base_url: str = DEFAULT_BASE
    timeout_seconds: float = 30.0
    max_retries: int = 4
    retry_backoff_base_seconds: float = 0.5


class CoinbaseProduct(BaseModel):
    product_id: str
    base_currency_id: str | None = None
    quote_currency_id: str | None = None
    status: str | None = None
    raw: dict[str, Any] = Field(default_factory=dict)


class CoinbaseCandle(BaseModel):
    start: datetime
    low: float
    high: float
    open: float
    close: float
    volume: float


class CoinbaseRESTClient:
    """Async HTTP client for Coinbase Advanced Trade (read-only market data)."""

    def __init__(self, settings: CoinbaseRESTSettings | None = None) -> None:
        self._settings = settings or CoinbaseRESTSettings()
        self._client = httpx.AsyncClient(
            base_url=self._settings.base_url.rstrip("/"),
            timeout=self._settings.timeout_seconds,
            headers={"Accept": "application/json"},
        )

    async def aclose(self) -> None:
        await self._client.aclose()

    async def _request_with_retry(self, method: str, url: str, **kwargs: Any) -> httpx.Response:
        import asyncio

        retryable = {429, 500, 502, 503, 504}
        last_err: BaseException | None = None
        for attempt in range(self._settings.max_retries):
            try:
                r = await self._client.request(method, url, **kwargs)
                if r.status_code in retryable:
                    last_err = httpx.HTTPStatusError(
                        f"HTTP {r.status_code}", request=r.request, response=r
                    )
                    if attempt == self._settings.max_retries - 1:
                        break
                    delay = self._settings.retry_backoff_base_seconds * (2**attempt)
                    delay += random.uniform(0, 0.1 * delay)
                    logger.warning(
                        "Coinbase REST retry %s/%s status=%s",
                        attempt + 1,
                        self._settings.max_retries,
                        r.status_code,
                    )
                    await asyncio.sleep(delay)
                    continue
                return r
            except httpx.TransportError as e:
                last_err = e
                if attempt == self._settings.max_retries - 1:
                    break
                delay = self._settings.retry_backoff_base_seconds * (2**attempt)
                delay += random.uniform(0, 0.1 * delay)
                logger.warning(
                    "Coinbase REST retry %s/%s transport %s",
                    attempt + 1,
                    self._settings.max_retries,
                    e,
                )
                await asyncio.sleep(delay)
        assert last_err is not None
        raise last_err

    async def list_products(self, limit: int = 250) -> list[CoinbaseProduct]:
        r = await self._request_with_retry("GET", "/products", params={"limit": limit})
        r.raise_for_status()
        body = r.json()
        products = body.get("products") or body.get("data") or []
        out: list[CoinbaseProduct] = []
        for p in products:
            if isinstance(p, dict) and "product_id" in p:
                out.append(
                    CoinbaseProduct(
                        product_id=p["product_id"],
                        base_currency_id=p.get("base_currency_id"),
                        quote_currency_id=p.get("quote_currency_id"),
                        status=p.get("status"),
                        raw=p,
                    )
                )
        return out

    async def get_public_candles(
        self,
        product_id: str,
        start: datetime,
        end: datetime,
        granularity_seconds: int,
    ) -> list[CoinbaseCandle]:
        """
        GET /products/{product_id}/candles — public market candles.

        granularity_seconds: e.g. 60, 300, 900, 3600, 21600, 86400
        """
        params = {
            "start": str(int(start.replace(tzinfo=UTC).timestamp())),
            "end": str(int(end.replace(tzinfo=UTC).timestamp())),
            "granularity": str(granularity_seconds),
        }
        path = f"/products/{product_id}/candles"
        r = await self._request_with_retry("GET", path, params=params)
        r.raise_for_status()
        body = r.json()
        candles_raw = body.get("candles") or body.get("data") or body
        if not isinstance(candles_raw, list):
            logger.warning("unexpected candles payload: %s", body)
            return []
        out: list[CoinbaseCandle] = []
        for row in candles_raw:
            if not isinstance(row, dict):
                continue
            ts = row.get("start") or row.get("time") or row.get("timestamp")
            if ts is None:
                continue
            start_dt = datetime.fromtimestamp(int(ts), tz=UTC)
            out.append(
                CoinbaseCandle(
                    start=start_dt,
                    low=float(row["low"]),
                    high=float(row["high"]),
                    open=float(row["open"]),
                    close=float(row["close"]),
                    volume=float(row.get("volume", 0)),
                )
            )
        out.sort(key=lambda c: c.start)
        return out
