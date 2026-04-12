"""FB-UX-004 Streamlit route guard env flag."""

from __future__ import annotations

import pytest

from control_plane import streamlit_util as su


def test_streamlit_route_guard_enabled_off_by_default(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("NM_STREAMLIT_ROUTE_GUARD_ENABLED", raising=False)
    assert su.streamlit_route_guard_enabled() is False


def test_streamlit_route_guard_enabled_true(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_STREAMLIT_ROUTE_GUARD_ENABLED", "true")
    assert su.streamlit_route_guard_enabled() is True
