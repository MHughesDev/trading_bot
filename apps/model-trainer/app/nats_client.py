import json
import os


class NatsPublisher:
    """Best-effort progress events.

    Every method already swallows a failed connection and a failed publish:
    progress is a convenience for a watching UI, and a trainer that refused to
    train because nobody was listening would be worse than a silent one.

    The import is inside `connect` for the same reason. `import nats` at module
    scope made the whole trainer — the adapters, the checkpoint producer, the
    resume path — unimportable wherever the client library is absent, which
    turned an optional notification channel into a hard dependency of training
    itself. A missing broker and a missing client library are the same fact from
    this class's point of view, and it now treats them the same way.
    """

    def __init__(self, url: str | None = None):
        self._url = url or os.environ.get("NATS_URL", "nats://localhost:4222")
        self._nc = None

    async def connect(self):
        try:
            import nats

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
