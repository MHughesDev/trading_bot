"""init_monitor toast edge detection (FB-UX-013)."""

from __future__ import annotations

from control_plane.init_monitor import should_emit_init_terminal_toast


def test_no_toast_without_prev() -> None:
    assert should_emit_init_terminal_toast(None, {"status": "succeeded"}) is False


def test_no_toast_if_already_terminal_prev() -> None:
    assert should_emit_init_terminal_toast("succeeded", {"status": "succeeded"}) is False


def test_toast_on_transition_to_succeeded() -> None:
    assert should_emit_init_terminal_toast("running", {"status": "succeeded"}) is True
