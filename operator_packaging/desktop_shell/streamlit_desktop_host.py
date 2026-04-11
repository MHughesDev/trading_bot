"""
Single-window desktop host for the Streamlit dashboard (FB-UI-02-02).

Loads ``NM_STREAMLIT_DESKTOP_URL`` (default ``http://127.0.0.1:8501``) in a native
webview. Requires **pywebview** (``pip install -e ".[dashboard]"``).

Run: ``python -m operator_packaging.desktop_shell``

Shell comparison (FB-UI-02-01): see ``docs/WINDOWS_OPERATOR_UI.MD`` §2.2.
"""

from __future__ import annotations

import os


def streamlit_desktop_url() -> str:
    """URL for the Streamlit app (not the control plane API)."""
    raw = os.getenv("NM_STREAMLIT_DESKTOP_URL", "").strip()
    if raw:
        return raw.rstrip("/")
    return "http://127.0.0.1:8501"


def window_title() -> str:
    t = os.getenv("NM_STREAMLIT_DESKTOP_TITLE", "NautilusMonster — Dashboard").strip()
    return t or "NautilusMonster — Dashboard"


def main() -> None:
    import webview

    url = streamlit_desktop_url()
    webview.create_window(window_title(), url, width=1280, height=800)
    webview.start()


if __name__ == "__main__":
    main()
