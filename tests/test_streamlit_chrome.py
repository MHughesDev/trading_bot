"""FB-UX-005 shared Streamlit chrome."""

from __future__ import annotations

from control_plane.streamlit_chrome import render_app_sidebar


def test_render_app_sidebar_is_callable() -> None:
    assert callable(render_app_sidebar)
