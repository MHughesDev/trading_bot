"""FB-UX-021: lifecycle header button morphing."""

from __future__ import annotations

from control_plane.asset_lifecycle_actions import lifecycle_action_spec


def test_header_action_spec_uninitialized() -> None:
    action, label, color = lifecycle_action_spec("uninitialized")
    assert action == "initialize"
    assert label == "Initialize"
    assert color == "#6366F1"


def test_header_action_spec_ready() -> None:
    action, label, color = lifecycle_action_spec("initialized_not_active")
    assert action == "start"
    assert label == "Start"
    assert color == "#22D3A0"


def test_header_action_spec_active() -> None:
    action, label, color = lifecycle_action_spec("active")
    assert action == "stop"
    assert label == "Stop"
    assert color == "#F87171"
