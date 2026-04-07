from __future__ import annotations

import json
from datetime import UTC, datetime
from typing import Any

import redis.asyncio as redis

from app.config.settings import RedisSettings


class RedisStore:
    def __init__(self, settings: RedisSettings) -> None:
        self._client = redis.Redis(
            host=settings.host,
            port=settings.port,
            db=settings.db,
            decode_responses=True,
        )

    async def publish_event(self, channel: str, payload: dict[str, Any]) -> None:
        await self._client.publish(channel, json.dumps(payload, default=str))

    async def set_live_state(
        self, key: str, payload: dict[str, Any], ttl_seconds: int = 30
    ) -> None:
        await self._client.set(key, json.dumps(payload, default=str), ex=ttl_seconds)

    async def get_live_state(self, key: str) -> dict[str, Any] | None:
        raw = await self._client.get(key)
        if raw is None:
            return None
        return json.loads(raw)

    async def heartbeat(self, service_name: str) -> None:
        key = f"heartbeat:{service_name}"
        await self.set_live_state(
            key,
            {"ts": datetime.now(UTC).isoformat()},
            ttl_seconds=10,
        )

    async def close(self) -> None:
        await self._client.aclose()
