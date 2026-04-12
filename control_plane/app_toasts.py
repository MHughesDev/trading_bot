"""In-app toasts (FB-UX-013) — optional ``st.toast`` behind ``NM_STREAMLIT_TOASTS_ENABLED``."""

from __future__ import annotations

import os


def streamlit_toasts_enabled() -> bool:
    return os.getenv("NM_STREAMLIT_TOASTS_ENABLED", "true").strip().lower() in (
        "1",
        "true",
        "yes",
    )


def maybe_toast(body: str, *, icon: str | None = None) -> None:
    """Fire a Streamlit toast if the dashboard extra is available and env allows it."""
    if not streamlit_toasts_enabled() or not (body or "").strip():
        return
    try:
        import streamlit as st
    except ImportError:
        return
    try:
        st.toast(body.strip(), icon=icon)
    except Exception:
        pass
