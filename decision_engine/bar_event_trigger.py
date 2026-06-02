"""Bar-close → AI decision trigger (the platform calls the AI, not vice-versa).

When a new canonical bar closes (the atomic minute data unit is added to the DB), the live
loop publishes a ``market.bar.closed.v1`` event onto the shared message bus. A
:class:`BarDecisionTrigger` subscribes to that topic and invokes a decision callback — so the
AI decision pipeline runs **in response to new data**, decoupled from how/where the AI lives.

This module depends only on the messaging layer and pure contracts, so it can be unit-tested
without QuestDB, a websocket, or the live loop. The decision callback is injected, which keeps
the trigger independent of the AI/policy implementation (which is a separate system).
"""

from __future__ import annotations

import logging
from collections.abc import Callable
from typing import Any

from pydantic import BaseModel

from shared.messaging.bus import MessageBus
from shared.messaging.envelope import EventEnvelope
from shared.messaging.topics import MARKET_BAR_CLOSED_V1

logger = logging.getLogger(__name__)

__all__ = [
    "BarClosedEvent",
    "publish_bar_closed",
    "BarDecisionTrigger",
    "MARKET_BAR_CLOSED_V1",
]


class BarClosedEvent(BaseModel):
    """Payload describing a freshly-closed canonical bar."""

    symbol: str
    ts: str  # ISO-8601 bar timestamp (bucket start)
    interval_seconds: int = 60
    open: float = 0.0
    high: float = 0.0
    low: float = 0.0
    close: float = 0.0
    volume: float = 0.0

    @classmethod
    def from_envelope(cls, envelope: EventEnvelope) -> "BarClosedEvent":
        return cls(**envelope.payload)


def publish_bar_closed(
    bus: MessageBus,
    *,
    symbol: str,
    ts: Any,
    interval_seconds: int = 60,
    open: float = 0.0,
    high: float = 0.0,
    low: float = 0.0,
    close: float = 0.0,
    volume: float = 0.0,
    trace_id: str | None = None,
    producer_service: str = "live_service",
) -> EventEnvelope:
    """Publish a ``market.bar.closed.v1`` event for a freshly-closed bar."""
    ts_iso = ts.isoformat() if hasattr(ts, "isoformat") else str(ts)
    event = BarClosedEvent(
        symbol=symbol,
        ts=ts_iso,
        interval_seconds=int(interval_seconds),
        open=float(open),
        high=float(high),
        low=float(low),
        close=float(close),
        volume=float(volume),
    )
    envelope = EventEnvelope(
        event_type=MARKET_BAR_CLOSED_V1,
        trace_id=trace_id or f"bar-{symbol}-{ts_iso}",
        producer_service=producer_service,
        symbol=symbol,
        partition_key=symbol,
        payload=event.model_dump(),
    )
    bus.publish(MARKET_BAR_CLOSED_V1, envelope)
    return envelope


class BarDecisionTrigger:
    """Subscribe to bar-closed events and run a decision callback for each new bar.

    The callback receives a :class:`BarClosedEvent`; what it does (run the canonical decision
    pipeline, notify an external agent, etc.) is the caller's choice — keeping this trigger
    independent of the AI implementation. Duplicate ``(symbol, ts)`` events are ignored by
    default so an at-least-once bus does not double-fire decisions. Callback exceptions are
    logged, never raised, so one bad bar cannot stall the bus.
    """

    def __init__(
        self,
        bus: MessageBus,
        decision_callback: Callable[[BarClosedEvent], Any],
        *,
        dedupe: bool = True,
    ) -> None:
        self._bus = bus
        self._callback = decision_callback
        self._dedupe = dedupe
        self._seen: set[tuple[str, str]] = set()
        self.processed_count = 0
        self.skipped_count = 0
        self.error_count = 0

    def start(self) -> None:
        """Register the handler on the bus (idempotent per instance)."""
        self._bus.subscribe(MARKET_BAR_CLOSED_V1, self._on_event)

    def _on_event(self, envelope: EventEnvelope) -> None:
        event = BarClosedEvent.from_envelope(envelope)
        key = (event.symbol, event.ts)
        if self._dedupe and key in self._seen:
            self.skipped_count += 1
            return
        self._seen.add(key)
        try:
            self._callback(event)
            self.processed_count += 1
        except Exception:
            self.error_count += 1
            logger.exception("bar decision callback failed symbol=%s ts=%s", event.symbol, event.ts)
