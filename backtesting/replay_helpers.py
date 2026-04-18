"""Helpers for canonical replay execution models (FB-CAN-009)."""

from __future__ import annotations

from app.contracts.replay_events import ExecutionModelProfile


def execution_profile_slippage_multiplier(profile: ExecutionModelProfile | str) -> float:
    p = str(profile)
    if p == "optimistic":
        return 0.85
    if p == "stress":
        return 1.35
    if p == "cascade_stress":
        return 1.6
    return 1.0


def execution_profile_fill_ratio(profile: ExecutionModelProfile | str) -> float:
    p = str(profile)
    if p == "optimistic":
        return 1.0
    if p == "stress":
        return 0.92
    if p == "cascade_stress":
        return 0.75
    return 1.0
