"""FB-UX-013 app_toasts."""

from __future__ import annotations

from control_plane.app_toasts import streamlit_toasts_enabled


def test_streamlit_toasts_enabled_default(monkeypatch) -> None:
    monkeypatch.delenv("NM_STREAMLIT_TOASTS_ENABLED", raising=False)
    assert streamlit_toasts_enabled() is True


def test_streamlit_toasts_disabled(monkeypatch) -> None:
    monkeypatch.setenv("NM_STREAMLIT_TOASTS_ENABLED", "false")
    assert streamlit_toasts_enabled() is False
