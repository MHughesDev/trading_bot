"""
Kraken public REST API — market data only (OHLC, trades, asset pairs).

Docs: https://docs.kraken.com/api/docs/rest-api
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

KRAKEN_API_BASE = "https://api.kraken.com"


class KrakenRESTSettings(BaseSettings):
    model_config = SettingsConfigDict(env_prefix="KRAKEN_REST_", env_file=".env", extra="ignore")

    base_url: str = KRAKEN_API_BASE
    timeout_seconds: float = 60.0
    max_retries: int = 4
    retry_backoff_base_seconds: float = 0.5


class KrakenAssetPair(BaseModel):
    altname: str
    wsname: str | None = None
    pair_decimals: int | None = None
    raw: dict[str, Any] = Field(default_factory=dict)


class KrakenOHLCRow(BaseModel):
    """One OHLC candle: time (sec UTC), o,h,l,c, vwap, volume, count."""

    time: int
    open: float
    high: float
    low: float
    close: float
    volume: float


class KrakenRESTClient:
    """Async client for Kraken public REST endpoints."""

    def __init__(self, settings: KrakenRESTSettings | None = None) -> None:
        self._settings = settings or KrakenRESTSettings()
        self._client = httpx.AsyncClient(
            base_url=self._settings.base_url.rstrip("/"),
            timeout=self._settings.timeout_seconds,
            headers={"Accept": "application/json"},
        )

    async def aclose(self) -> None:
        await self._client.aclose()

    async def _request(self, path: str, params: dict[str, Any]) -> dict[str, Any]:
        import asyncio

        retryable = {429, 500, 502, 503, 504}
        last_err: BaseException | None = None
        for attempt in range(self._settings.max_retries):
            try:
                r = await self._client.get(path, params=params)
                if r.status_code in retryable:
                    last_err = httpx.HTTPStatusError(
                        f"HTTP {r.status_code}", request=r.request, response=r
                    )
                    if attempt < self._settings.max_retries - 1:
                        delay = self._settings.retry_backoff_base_seconds * (2**attempt)
                        delay += random.uniform(0, 0.1 * delay)
                        await asyncio.sleep(delay)
                        continue
                r.raise_for_status()
                body = r.json()
                if body.get("error"):
                    raise RuntimeError(f"Kraken API error: {body['error']}")
                return body
            except httpx.TransportError as e:
                last_err = e
                if attempt < self._settings.max_retries - 1:
                    delay = self._settings.retry_backoff_base_seconds * (2**attempt)
                    await asyncio.sleep(delay)
                    continue
                raise
        assert last_err is not None
        raise last_err

    async def list_asset_pairs(self) -> dict[str, KrakenAssetPair]:
        body = await self._request("/0/public/AssetPairs", {})
        result = body.get("result") or {}
        out: dict[str, KrakenAssetPair] = {}
        for k, v in result.items():
            if not isinstance(v, dict):
                continue
            alt = v.get("altname") or k
            ws = v.get("wsname")
            pd_ = v.get("pair_decimals")
            out[k] = KrakenAssetPair(
                altname=str(alt),
                wsname=str(ws) if ws else None,
                pair_decimals=int(pd_) if pd_ is not None else None,
                raw=v,
            )
        return out

    async def ohlc(
        self,
        pair: str,
        *,
        interval_minutes: int,
        since: int | None = None,
    ) -> tuple[list[KrakenOHLCRow], int | None]:
        """
        GET /OHLC — up to ~720 candles per call.

        ``interval_minutes`` must be one of: 1, 5, 15, 30, 60, 240, 1440, 10080, 21600.
        ``since``: unix timestamp (seconds) for earliest desired candle (API-specific).
        """
        params: dict[str, Any] = {"pair": pair, "interval": interval_minutes}
        if since is not None:
            params["since"] = since
        body = await self._request("/0/public/OHLC", params)
        result = body.get("result") or {}
        last = result.get("last")
        # result key is pair name (may differ from request)
        rows_raw: list[Any] = []
        for k, v in result.items():
            if k == "last":
                continue
            if isinstance(v, list):
                rows_raw = v
                break
        out: list[KrakenOHLCRow] = []
        for row in rows_raw:
            if not isinstance(row, (list, tuple)) or len(row) < 8:
                continue
            out.append(
                KrakenOHLCRow(
                    time=int(row[0]),
                    open=float(row[1]),
                    high=float(row[2]),
                    low=float(row[3]),
                    close=float(row[4]),
                    volume=float(row[6]),
                )
            )
        out.sort(key=lambda x: x.time)
        last_int = int(last) if last is not None else None
        return out, last_int

    async def ticker_mid(self, pair: str) -> float | None:
        """
        GET /Ticker — best bid/ask mid for ``pair`` (REST pair name, e.g. ``XBTUSD``).

        Returns ``None`` if the pair is missing or bid/ask cannot be parsed.
        """
        body = await self._request("/0/public/Ticker", {"pair": pair})
        result = body.get("result") or {}
        for k, v in result.items():
            if k == "last" or not isinstance(v, dict):
                continue
            a = v.get("a")
            b = v.get("b")
            if not isinstance(a, (list, tuple)) or not isinstance(b, (list, tuple)):
                continue
            if len(a) < 1 or len(b) < 1:
                continue
            try:
                ask = float(a[0])
                bid = float(b[0])
            except (TypeError, ValueError):
                continue
            return (ask + bid) / 2.0
        return None


def granularity_to_kraken_ohlc_interval_minutes(granularity_seconds: int) -> int | None:
    """
    Map bar size in seconds to Kraken OHLC ``interval`` (minutes), if exact match.

    Kraken supports minutes: 1,5,15,30,60,240,1440,10080,21600 only.
    """
    allowed = (1, 5, 15, 30, 60, 240, 1440, 10080, 21600)
    for m in allowed:
        if granularity_seconds == m * 60:
            return m
    return None


async def fetch_ohlc_range(
    client: KrakenRESTClient,
    pair: str,
    start: datetime,
    end: datetime,
    *,
    interval_minutes: int,
    max_iterations: int = 10_000,
) -> list[KrakenOHLCRow]:
    """
    Paginate ``/OHLC`` with ``since`` until ``[start, end)`` is covered (~720 candles per call).
    """
    start = start.replace(tzinfo=UTC) if start.tzinfo is None else start.astimezone(UTC)
    end = end.replace(tzinfo=UTC) if end.tzinfo is None else end.astimezone(UTC)
    start_ts = int(start.timestamp())
    end_ts = int(end.timestamp())
    interval_sec = interval_minutes * 60

    all_rows: list[KrakenOHLCRow] = []
    seen: set[int] = set()
    since = start_ts
    it = 0
    while since < end_ts and it < max_iterations:
        it += 1
        batch, _last = await client.ohlc(pair, interval_minutes=interval_minutes, since=since)
        if not batch:
            break
        for c in batch:
            if start_ts <= c.time < end_ts and c.time not in seen:
                seen.add(c.time)
                all_rows.append(c)
        last_t = max(c.time for c in batch)
        nxt = last_t + interval_sec
        if nxt <= since:
            break
        since = nxt
        if len(batch) < 2:
            break
    all_rows.sort(key=lambda x: x.time)
    return all_rows


async def fetch_trades_range(
    client: KrakenRESTClient,
    pair: str,
    start: datetime,
    end: datetime,
    *,
    max_batches: int = 50_000,
) -> list[tuple[float, float, float]]:
    """
    Paginate ``/Trades`` (price, volume, time in seconds with fractional part).

    ``since`` is Kraken's nanosecond id string (from ``last`` in the response).
    """
    start = start.replace(tzinfo=UTC) if start.tzinfo is None else start.astimezone(UTC)
    end = end.replace(tzinfo=UTC) if end.tzinfo is None else end.astimezone(UTC)
    start_ns = str(int(start.timestamp() * 1e9))
    end_ts = end.timestamp()

    trades: list[tuple[float, float, float]] = []
    since: str | int = start_ns
    batches = 0
    prev_since: str | None = None
    while batches < max_batches:
        batches += 1
        params: dict[str, Any] = {"pair": pair, "since": since}
        body = await client._request("/0/public/Trades", params)
        result = body.get("result") or {}
        last = result.get("last")
        trade_rows: list[Any] = []
        for k, v in result.items():
            if k == "last":
                continue
            if isinstance(v, list):
                trade_rows = v
                break
        if not trade_rows:
            break
        for tr in trade_rows:
            if not isinstance(tr, (list, tuple)) or len(tr) < 3:
                continue
            price = float(tr[0])
            vol = float(tr[1])
            t = float(tr[2])
            if t >= start.timestamp() and t < end_ts:
                trades.append((price, vol, t))
        if last is None or str(last) == prev_since:
            break
        prev_since = str(last)
        since = last
    return trades
