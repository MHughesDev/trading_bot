"""Lifecycle UI helpers for Asset page (FB-AP-031) — testable without Streamlit."""

from __future__ import annotations

import time
from typing import Any, Callable, Literal

LifecyclePrimaryAction = Literal["initialize", "start", "stop"]


def lifecycle_action_spec(lifecycle_state: str) -> tuple[LifecyclePrimaryAction | None, str, str]:
    """Return ``(action, label, color_hex)`` for Asset header morphing button."""
    action = primary_lifecycle_action(lifecycle_state)
    if action == "initialize":
        return action, "Initialize", "#6366F1"
    if action == "start":
        return action, "Start", "#22D3A0"
    if action == "stop":
        return action, "Stop", "#F87171"
    return None, "—", "#9CA3AF"


def primary_lifecycle_action(lifecycle_state: str) -> LifecyclePrimaryAction | None:
    """
    Map persisted lifecycle to the single primary operator action for this state.

    States from :func:`app.runtime.asset_lifecycle_state.effective_lifecycle_state`.
    """
    s = lifecycle_state.strip().lower()
    if s == "uninitialized":
        return "initialize"
    if s == "initialized_not_active":
        return "start"
    if s == "active":
        return "stop"
    return None


def init_job_is_terminal(job: dict[str, Any]) -> bool:
    st = str(job.get("status") or "").lower()
    return st in ("succeeded", "failed")


def poll_init_job_until_terminal(
    job_id: str,
    *,
    fetch_job: Callable[[str], dict[str, Any]],
    sleep_s: float = 0.15,
    max_iterations: int = 10_000,
) -> dict[str, Any]:
    """
    Poll ``GET /assets/init/jobs/{job_id}`` until status is terminal or iteration cap hit.

    Used by the Asset page during initialization; cap avoids infinite spin if API misbehaves.
    """
    last: dict[str, Any] = {}
    for _ in range(max_iterations):
        last = fetch_job(job_id)
        if init_job_is_terminal(last):
            return last
        time.sleep(sleep_s)
    return last
