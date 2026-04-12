"""Per-asset active watch gating for the live loop (FB-AP-038)."""

from __future__ import annotations

import time
from collections.abc import Callable

from app.config.settings import AppSettings
from app.contracts.asset_lifecycle import AssetLifecycleState


def should_run_decision_tick(
    symbol: str,
    settings: AppSettings,
    *,
    effective_lifecycle: Callable[[str], AssetLifecycleState],
    last_decision_monotonic: dict[str, float],
) -> bool:
    """
    Return True if this tick should run ``run_decision_tick`` (pipeline + risk) for ``symbol``.

    When ``live_watch_lifecycle_gate`` is True, only ``active`` lifecycle symbols run inference.
    When ``live_decision_min_interval_seconds`` > 0, enforce a minimum wall interval per symbol
    using monotonic time between **successful** decision ticks (throttle).
    """
    sym = symbol.strip()
    if settings.live_watch_lifecycle_gate:
        if effective_lifecycle(sym) != AssetLifecycleState.active:
            return False
    min_s = float(settings.live_decision_min_interval_seconds)
    if min_s > 0:
        now = time.monotonic()
        prev = last_decision_monotonic.get(sym)
        if prev is not None and (now - prev) < min_s:
            return False
    return True


def record_decision_tick(symbol: str, last_decision_monotonic: dict[str, float]) -> None:
    """Call after ``run_decision_tick`` when the tick was not skipped."""
    last_decision_monotonic[symbol.strip()] = time.monotonic()
