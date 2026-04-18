"""FB-UX-019: global CSS injection contract."""

from __future__ import annotations

import sys
import types

from control_plane import _theme


def test_inject_global_css_contract(monkeypatch):
    calls: list[tuple[str, bool]] = []

    fake_st = types.SimpleNamespace()

    def _markdown(payload: str, *, unsafe_allow_html: bool = False):
        calls.append((payload, unsafe_allow_html))

    fake_st.markdown = _markdown
    monkeypatch.setitem(sys.modules, "streamlit", fake_st)

    css = _theme.inject_global_css()
    assert "fonts.googleapis.com" in css
    assert "Inter" in css
    assert "JetBrains Mono" in css
    assert "font-variant-numeric: tabular-nums" in css
    assert "#MainMenu" in css
    assert "footer" in css
    assert "header" in css
    assert "--pnl-up: #22D3A0" in css
    assert "--pnl-down: #F87171" in css
    assert calls and calls[0][1] is True
