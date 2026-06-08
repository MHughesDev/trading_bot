"""
Single-window desktop host for the React trading bot UI.

Loads ``NM_DESKTOP_URL`` (default ``http://127.0.0.1:8001``) in a native webview.
Requires **pywebview** (``pip install -e ".[dashboard]"``).

Run: ``python -m operator_packaging.desktop_shell``
"""

from __future__ import annotations

import os


def desktop_url() -> str:
    """URL for the React SPA served by FastAPI."""
    raw = os.getenv("NM_DESKTOP_URL", "").strip()
    if raw:
        return raw.rstrip("/")
    return "http://127.0.0.1:8001"


def window_title() -> str:
    t = os.getenv("NM_DESKTOP_TITLE", "Trading Bot").strip()
    return t or "Trading Bot"


def main() -> None:
    import webview

    url = desktop_url()
    webview.create_window(window_title(), url, width=1280, height=800)
    webview.start()


if __name__ == "__main__":
    main()
