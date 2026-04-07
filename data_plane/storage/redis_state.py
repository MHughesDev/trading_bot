"""Redis: live state and pub/sub channels."""

from __future__ import annotations

import json
from typing import Any

import redis.asyncio as redis

from app.contracts.events import BarEvent


class RedisState:
    def __init__(self, url: str, bar_ttl_seconds: int = 86_400) -> None:
        self._url = url
        self._bar_ttl = bar_ttl_seconds
        self._r: redis.Redis | None = None

    async def connect(self) -> None:
        self._r = redis.from_url(self._url, decode_responses=True)

    async def aclose(self) -> None:
        if self._r:
            await self._r.aclose()
            self._r = None

    def _key_bar(self, symbol: str) -> str:
        return f"nm:bar:{symbol}"

    def _key_mode(self) -> str:
        return "nm:system:mode"

    async def set_latest_bar(self, bar: BarEvent) -> None:
        if not self._r:
            raise RuntimeError("not connected")
        await self._r.set(
            self._key_bar(bar.symbol),
            bar.model_dump_json(),
            ex=self._bar_ttl,
        )
        await self._r.publish("nm:bars", json.dumps({"symbol": bar.symbol}))

    async def get_latest_bar(self, symbol: str) -> BarEvent | None:
        if not self._r:
            raise RuntimeError("not connected")
        raw = await self._r.get(self._key_bar(symbol))
        if not raw:
            return None
        data = json.loads(raw)
        return BarEvent.model_validate(data)

    async def set_kv(self, key: str, value: Any) -> None:
        if not self._r:
            raise RuntimeError("not connected")
        await self._r.set(key, json.dumps(value))

    async def get_kv(self, key: str) -> Any | None:
        if not self._r:
            raise RuntimeError("not connected")
        raw = await self._r.get(key)
        return json.loads(raw) if raw else None

    async def publish(self, channel: str, message: dict[str, Any]) -> None:
        if not self._r:
            raise RuntimeError("not connected")
        await self._r.publish(channel, json.dumps(message, default=str))
