"""Tests for Asset page lifecycle action helpers (FB-AP-031)."""

from __future__ import annotations

from control_plane.asset_lifecycle_actions import (
    init_job_is_terminal,
    poll_init_job_until_terminal,
    primary_lifecycle_action,
)


def test_primary_lifecycle_action() -> None:
    assert primary_lifecycle_action("uninitialized") == "initialize"
    assert primary_lifecycle_action("initialized_not_active") == "start"
    assert primary_lifecycle_action("active") == "stop"
    assert primary_lifecycle_action("  ACTIVE ") == "stop"
    assert primary_lifecycle_action("unknown") is None


def test_init_job_is_terminal() -> None:
    assert init_job_is_terminal({"status": "succeeded"})
    assert init_job_is_terminal({"status": "failed"})
    assert not init_job_is_terminal({"status": "running"})


def test_poll_init_job_until_terminal() -> None:
    calls: list[str] = []

    def fetch(job_id: str) -> dict:
        calls.append(job_id)
        if len(calls) < 3:
            return {"status": "running"}
        return {"status": "succeeded"}

    out = poll_init_job_until_terminal(
        "j1",
        fetch_job=fetch,
        sleep_s=0.0,
        max_iterations=20,
    )
    assert out["status"] == "succeeded"
    assert len(calls) == 3
