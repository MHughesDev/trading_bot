import json
import os
import nats


class NatsPublisher:
    def __init__(self, url: str | None = None):
        self._url = url or os.environ.get("NATS_URL", "nats://localhost:4222")
        self._nc = None

    async def connect(self):
        try:
            self._nc = await nats.connect(self._url)
        except Exception:
            self._nc = None

    async def publish(self, subject: str, payload: dict):
        if self._nc is None:
            return
        try:
            await self._nc.publish(subject, json.dumps(payload).encode())
        except Exception:
            pass

    async def close(self):
        if self._nc is not None:
            try:
                await self._nc.drain()
            except Exception:
                pass
