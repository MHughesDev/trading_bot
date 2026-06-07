"""Tests for the bar-close → AI decision trigger (platform triggers the AI)."""

from __future__ import annotations

from datetime import UTC, datetime

from app.runtime.bar_event_trigger import (
    MARKET_BAR_CLOSED_V1,
    BarClosedEvent,
    BarDecisionTrigger,
    publish_bar_closed,
)
from shared.messaging.in_memory import InMemoryMessageBus


def test_publish_then_trigger_invokes_callback() -> None:
    bus = InMemoryMessageBus()
    seen: list[BarClosedEvent] = []
    trigger = BarDecisionTrigger(bus, seen.append)
    trigger.start()

    publish_bar_closed(
        bus,
        symbol="BTC-USD",
        ts=datetime(2025, 1, 1, 0, 1, tzinfo=UTC),
        interval_seconds=60,
        open=100.0,
        high=101.0,
        low=99.0,
        close=100.5,
        volume=12.0,
    )

    assert trigger.processed_count == 1
    assert len(seen) == 1
    ev = seen[0]
    assert ev.symbol == "BTC-USD"
    assert ev.interval_seconds == 60
    assert ev.close == 100.5
    assert ev.ts == "2025-01-01T00:01:00+00:00"


def test_dedupe_skips_repeat_same_bar() -> None:
    bus = InMemoryMessageBus()
    calls: list[BarClosedEvent] = []
    trigger = BarDecisionTrigger(bus, calls.append)
    trigger.start()
    ts = datetime(2025, 1, 1, 0, 1, tzinfo=UTC)
    publish_bar_closed(bus, symbol="BTC-USD", ts=ts)
    publish_bar_closed(bus, symbol="BTC-USD", ts=ts)  # duplicate
    assert trigger.processed_count == 1
    assert trigger.skipped_count == 1
    assert len(calls) == 1


def test_distinct_bars_each_trigger() -> None:
    bus = InMemoryMessageBus()
    calls: list[BarClosedEvent] = []
    BarDecisionTrigger(bus, calls.append).start()
    publish_bar_closed(bus, symbol="BTC-USD", ts=datetime(2025, 1, 1, 0, 1, tzinfo=UTC))
    publish_bar_closed(bus, symbol="BTC-USD", ts=datetime(2025, 1, 1, 0, 2, tzinfo=UTC))
    publish_bar_closed(bus, symbol="ETH-USD", ts=datetime(2025, 1, 1, 0, 1, tzinfo=UTC))
    assert len(calls) == 3


def test_callback_exception_is_isolated() -> None:
    bus = InMemoryMessageBus()

    def _boom(_ev: BarClosedEvent) -> None:
        raise RuntimeError("decision failed")

    trigger = BarDecisionTrigger(bus, _boom)
    trigger.start()
    publish_bar_closed(bus, symbol="BTC-USD", ts=datetime(2025, 1, 1, 0, 1, tzinfo=UTC))
    assert trigger.error_count == 1
    assert trigger.processed_count == 0  # bus did not raise


def test_publish_returns_envelope_with_topic() -> None:
    bus = InMemoryMessageBus()
    env = publish_bar_closed(bus, symbol="SOL-USD", ts="2025-01-01T00:01:00+00:00")
    assert env.event_type == MARKET_BAR_CLOSED_V1
    assert env.symbol == "SOL-USD"
    assert BarClosedEvent.from_envelope(env).symbol == "SOL-USD"
