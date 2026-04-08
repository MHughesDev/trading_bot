"""
Coinbase Advanced Trade REST client (market data ONLY).

Public endpoints do not require authentication. Signed requests for private trading use CDP keys.
https://docs.cdp.coinbase.com/advanced-trade/docs/rest-api

If Advanced Trade returns 401/403 without JWT, list_products and get_public_candles fall back to
the legacy Coinbase Exchange public API (same product_id format for BTC-USD, etc.).
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
# Public Exchange API (no auth) — fallback when Advanced Trade returns 401/403 without JWT (Issue 35)
EXCHANGE_PUBLIC_BASE = "https://api.exchange.coinbase.com"


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
        self._exchange = httpx.AsyncClient(
            base_url=EXCHANGE_PUBLIC_BASE,
            timeout=self._settings.timeout_seconds,
            headers={"Accept": "application/json"},
        )

    async def aclose(self) -> None:
        await self._client.aclose()
        await self._exchange.aclose()

    async def _request_with_retry(
        self,
        method: str,
        url: str,
        *,
        client: httpx.AsyncClient | None = None,
        **kwargs: Any,
    ) -> httpx.Response:
        import asyncio

        http = client or self._client
        retryable = {429, 500, 502, 503, 504}
        last_err: BaseException | None = None
        for attempt in range(self._settings.max_retries):
            try:
                r = await http.request(method, url, **kwargs)
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

    def _parse_products_payload(self, body: Any) -> list[CoinbaseProduct]:
        if isinstance(body, list):
            products = body
        else:
            products = body.get("products") or body.get("data") or [] if isinstance(body, dict) else []
        out: list[CoinbaseProduct] = []
        for p in products:
            if not isinstance(p, dict):
                continue
            pid = p.get("product_id") or p.get("id")
            if not pid:
                continue
            out.append(
                CoinbaseProduct(
                    product_id=str(pid),
                    base_currency_id=p.get("base_currency_id"),
                    quote_currency_id=p.get("quote_currency_id"),
                    status=p.get("status") or p.get("trading_disabled"),
                    raw=p,
                )
            )
        return out

    async def list_products(self, limit: int = 250) -> list[CoinbaseProduct]:
        r = await self._request_with_retry("GET", "/products", params={"limit": limit})
        if r.status_code in (401, 403):
            logger.warning(
                "Coinbase Advanced Trade GET /products returned %s; using Exchange public /products (no JWT)",
                r.status_code,
            )
            er = await self._request_with_retry("GET", "/products", client=self._exchange)
            er.raise_for_status()
            return self._parse_products_payload(er.json())
        r.raise_for_status()
        return self._parse_products_payload(r.json())

    def _parse_exchange_candles(self, raw: Any) -> list[CoinbaseCandle]:
        """Parse Coinbase Exchange API candle rows: [time, low, high, open, close, volume]."""
        if not isinstance(raw, list):
            return []
        out: list[CoinbaseCandle] = []
        for row in raw:
            if not isinstance(row, (list, tuple)) or len(row) < 6:
                continue
            ts = int(row[0])
            start_dt = datetime.fromtimestamp(ts, tz=UTC)
            out.append(
                CoinbaseCandle(
                    start=start_dt,
                    low=float(row[1]),
                    high=float(row[2]),
                    open=float(row[3]),
                    close=float(row[4]),
                    volume=float(row[5]),
                )
            )
        out.sort(key=lambda c: c.start)
        return out

    async def _get_exchange_candles(
        self,
        product_id: str,
        start: datetime,
        end: datetime,
        granularity_seconds: int,
    ) -> list[CoinbaseCandle]:
        """Public Exchange candles (no JWT) — Issue 35 fallback."""
        params = {
            "start": start.replace(tzinfo=UTC).isoformat().replace("+00:00", "Z"),
            "end": end.replace(tzinfo=UTC).isoformat().replace("+00:00", "Z"),
            "granularity": str(granularity_seconds),
        }
        path = f"/products/{product_id}/candles"
        r = await self._request_with_retry("GET", path, params=params, client=self._exchange)
        r.raise_for_status()
        body = r.json()
        return self._parse_exchange_candles(body)

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
        if r.status_code in (401, 403):
            logger.warning(
                "Coinbase Advanced Trade GET %s returned %s; using Exchange public candles (no JWT)",
                path,
                r.status_code,
            )
            return await self._get_exchange_candles(product_id, start, end, granularity_seconds)
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
