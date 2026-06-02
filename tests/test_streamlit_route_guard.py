"""FB-UX-004 Streamlit route guard is always on."""

from __future__ import annotations

from control_plane import streamlit_util as su


def test_streamlit_route_guard_enabled_is_always_true() -> None:
    assert su.streamlit_route_guard_enabled() is True
