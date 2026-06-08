"""Bar-close decision trigger: settings flag, lifecycle gating, and pending-event wiring."""

from __future__ import annotations

from datetime import UTC, datetime

import pytest

from app.config.settings import AppSettings, _yaml_to_kwargs
from app.contracts.asset_lifecycle import AssetLifecycleState
from app.runtime.live_watch_gate import lifecycle_allows_decision
from app.runtime.bar_event_trigger import BarClosedEvent, BarDecisionTrigger, publish_bar_closed
from shared.messaging.in_memory import InMemoryMessageBus


def test_flag_default_off_and_yaml_maps() -> None:
    assert AppSettings().bar_close_decision_trigger_enabled is False
    kw = _yaml_to_kwargs({"microservices": {"bar_close_decision_trigger_enabled": True}})
    assert kw["bar_close_decision_trigger_enabled"] is True


def test_flag_env_override(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_BAR_CLOSE_DECISION_TRIGGER_ENABLED", "true")
    assert AppSettings().bar_close_decision_trigger_enabled is True


def test_lifecycle_allows_decision() -> None:
    gated = AppSettings(live_watch_lifecycle_gate=True)
    ungated = AppSettings(live_watch_lifecycle_gate=False)

    def active(_s: str) -> AssetLifecycleState:
        return AssetLifecycleState.active

    def idle(_s: str) -> AssetLifecycleState:
        return AssetLifecycleState.initialized_not_active

    assert lifecycle_allows_decision("BTC-USD", gated, effective_lifecycle=active) is True
    assert lifecycle_allows_decision("BTC-USD", gated, effective_lifecycle=idle) is False
    # Gate off: lifecycle is irrelevant.
    assert lifecycle_allows_decision("BTC-USD", ungated, effective_lifecycle=idle) is True


def test_trigger_records_pending_and_dedupes() -> None:
    # Mirrors the live_service wiring: a closed bar records a pending decision for its symbol.
    bus = InMemoryMessageBus()
    pending: dict[str, BarClosedEvent] = {}
    trigger = BarDecisionTrigger(bus, lambda ev: pending.__setitem__(ev.symbol, ev))
    trigger.start()

    ts = datetime(2026, 6, 2, 12, 0, tzinfo=UTC)
    publish_bar_closed(bus, symbol="BTC-USD", ts=ts, close=65000.0)
    assert "BTC-USD" in pending
    assert pending["BTC-USD"].close == 65000.0
    assert trigger.processed_count == 1

    # The loop consumes the pending event.
    assert pending.pop("BTC-USD", None) is not None

    # Same (symbol, ts) is a duplicate → skipped, not re-fired.
    publish_bar_closed(bus, symbol="BTC-USD", ts=ts, close=65000.0)
    assert trigger.skipped_count == 1
    assert "BTC-USD" not in pending

    # A new bar (new ts) fires again.
    publish_bar_closed(bus, symbol="BTC-USD", ts=datetime(2026, 6, 2, 12, 1, tzinfo=UTC), close=65010.0)
    assert pending["BTC-USD"].close == 65010.0
