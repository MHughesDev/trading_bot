"""The trainer's bounded telemetry queue (SPEC 16.1, checklist 5.1, AT-55).

A training loop emits progress far faster than a websocket can deliver it, and
the two ways that usually ends are both bad: an unbounded queue turns a slow
consumer into an out-of-memory kill of the training run, and a blocking send
turns it into a GPU stalling on a socket.

So the queue is bounded and **drops the oldest**. Not the newest — dropping the
newest keeps a stale view that never catches up, which is worse than useless on
a progress display because it looks live. Dropping the oldest means a slow
consumer sees a gap and then the present.

The drop count is reported. A queue that silently drops is a queue whose
consumer cannot tell "training is quiet" from "telemetry is broken", and those
need different responses.
"""

from __future__ import annotations

import asyncio
from collections import deque
from dataclasses import dataclass, field
from typing import Any

# Deep enough to ride out a few seconds of a stalled consumer at the 5 s
# progress rate limit; shallow enough that the memory is irrelevant next to a
# model.
DEFAULT_CAPACITY = 256


@dataclass
class BoundedTelemetry:
    """Drop-oldest queue with a drop counter."""

    capacity: int = DEFAULT_CAPACITY
    _items: deque = field(default_factory=deque, init=False)
    dropped: int = field(default=0, init=False)

    def __post_init__(self) -> None:
        if self.capacity < 1:
            raise ValueError("a telemetry queue needs room for at least one message")
        self._items = deque(maxlen=self.capacity)

    def publish(self, message: Any) -> None:
        """Enqueue, dropping the oldest if full.

        Never blocks and never raises. The training loop calling this must not be
        able to be slowed down, let alone stopped, by a display.
        """
        if len(self._items) == self.capacity:
            self.dropped += 1
        self._items.append(message)

    def drain(self) -> list[Any]:
        """Take everything queued. The drop counter is *not* reset — it is a
        property of the stream, and resetting it on read would hide a consumer
        that is chronically behind."""
        out = list(self._items)
        self._items.clear()
        return out

    def snapshot(self) -> dict[str, Any]:
        """What the consumer needs to know about its own view.

        `dropped` travels with the messages, because a gap the consumer cannot
        see is a gap it will interpret as quiet.
        """
        return {"queued": len(self._items), "dropped": self.dropped, "capacity": self.capacity}

    def is_lagging(self) -> bool:
        """Whether anything has been lost. One dropped message is lagging."""
        return self.dropped > 0


async def pump(queue: BoundedTelemetry, send, interval_s: float = 1.0) -> None:
    """Drain into `send` on a timer.

    Errors from `send` are swallowed deliberately: a broken display must not
    stop a training run, and the drop counter already records what a broken
    display costs.
    """
    while True:
        batch = queue.drain()
        if batch:
            try:
                await send(batch, queue.snapshot())
            except Exception:  # noqa: BLE001
                pass
        await asyncio.sleep(interval_s)
