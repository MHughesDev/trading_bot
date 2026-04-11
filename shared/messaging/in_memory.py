"""In-memory message bus for local development and tests."""

from __future__ import annotations

from collections import defaultdict

from shared.messaging.bus import MessageBus, MessageHandler
from shared.messaging.envelope import EventEnvelope


class InMemoryMessageBus(MessageBus):
    """Simple pub/sub bus with in-process handlers and DLQ support.

    This implementation is intentionally lightweight for local/dev execution.
    """

    def __init__(self, *, max_retries: int = 2) -> None:
        self._handlers: dict[str, list[MessageHandler]] = defaultdict(list)
        self._dlq: dict[str, list[EventEnvelope]] = defaultdict(list)
        self._max_retries = max(0, int(max_retries))

    def publish(self, topic: str, envelope: EventEnvelope) -> None:
        handlers = self._handlers.get(topic, [])
        for handler in handlers:
            attempts = 0
            while True:
                try:
                    handler(envelope)
                    break
                except Exception:  # noqa: BLE001 - keep lightweight local reliability loop
                    attempts += 1
                    if attempts > self._max_retries:
                        self._dlq[topic].append(envelope)
                        break

    def subscribe(self, topic: str, handler: MessageHandler) -> None:
        self._handlers[topic].append(handler)

    def dead_letter(self, topic: str) -> list[EventEnvelope]:
        """Return dead-lettered events for a topic."""
        return list(self._dlq.get(topic, []))
