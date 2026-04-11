"""operator_packaging.desktop_shell URL helpers (no pywebview import)."""

from __future__ import annotations

import pytest

from operator_packaging.desktop_shell import streamlit_desktop_host as h


def test_streamlit_desktop_url_default(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("NM_STREAMLIT_DESKTOP_URL", raising=False)
    assert h.streamlit_desktop_url() == "http://127.0.0.1:8501"


def test_streamlit_desktop_url_custom(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_STREAMLIT_DESKTOP_URL", "http://localhost:9000/")
    assert h.streamlit_desktop_url() == "http://localhost:9000"


def test_window_title_default(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("NM_STREAMLIT_DESKTOP_TITLE", raising=False)
    assert "NautilusMonster" in h.window_title()


def test_window_title_custom(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_STREAMLIT_DESKTOP_TITLE", "Test")
    assert h.window_title() == "Test"
