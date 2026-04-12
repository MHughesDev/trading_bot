"""Init pipeline monitor helpers (FB-UX-009)."""

from __future__ import annotations

from control_plane.init_monitor import _step_summary


def test_step_summary_empty() -> None:
    assert _step_summary([]) == "—"


def test_step_summary_last_step() -> None:
    s = _step_summary(
        [
            {"step": "kraken_fetch", "status": "done"},
            {"step": "validate", "status": "done"},
        ]
    )
    assert "validate" in s
    assert "done" in s
