"""FB-AP-038 active watch gating."""

from __future__ import annotations

import pytest

from app.config.settings import AppSettings
from app.contracts.asset_lifecycle import AssetLifecycleState
from app.runtime.live_watch_gate import record_decision_tick, should_run_decision_tick


def test_lifecycle_gate_blocks_non_active() -> None:
    s = AppSettings(live_watch_lifecycle_gate=True)

    def eff(_sym: str) -> AssetLifecycleState:
        return AssetLifecycleState.initialized_not_active

    last: dict[str, float] = {}
    assert not should_run_decision_tick("BTC-USD", s, effective_lifecycle=eff, last_decision_monotonic=last)


def test_lifecycle_gate_allows_active() -> None:
    s = AppSettings(live_watch_lifecycle_gate=True)

    def eff(_sym: str) -> AssetLifecycleState:
        return AssetLifecycleState.active

    last: dict[str, float] = {}
    assert should_run_decision_tick("BTC-USD", s, effective_lifecycle=eff, last_decision_monotonic=last)


def test_min_interval_throttles_second_tick() -> None:
    s = AppSettings(
        live_watch_lifecycle_gate=False,
        live_decision_min_interval_seconds=10.0,
    )

    def eff(_sym: str) -> AssetLifecycleState:
        return AssetLifecycleState.active

    last: dict[str, float] = {}
    assert should_run_decision_tick("BTC-USD", s, effective_lifecycle=eff, last_decision_monotonic=last)
    record_decision_tick("BTC-USD", last)
    assert not should_run_decision_tick("BTC-USD", s, effective_lifecycle=eff, last_decision_monotonic=last)


def test_min_interval_allows_after_elapsed(monkeypatch: pytest.MonkeyPatch) -> None:
    s = AppSettings(
        live_watch_lifecycle_gate=False,
        live_decision_min_interval_seconds=0.5,
    )

    def eff(_sym: str) -> AssetLifecycleState:
        return AssetLifecycleState.active

    last: dict[str, float] = {}
    t = [0.0]

    def fake_mono() -> float:
        return t[0]

    monkeypatch.setattr("app.runtime.live_watch_gate.time.monotonic", fake_mono)
    assert should_run_decision_tick("X", s, effective_lifecycle=eff, last_decision_monotonic=last)
    record_decision_tick("X", last)
    t[0] = 1.0
    assert should_run_decision_tick("X", s, effective_lifecycle=eff, last_decision_monotonic=last)
