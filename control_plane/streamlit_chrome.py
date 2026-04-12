"""Shared Streamlit sidebar chrome (FB-UX-005 — Account link + navigation)."""

from __future__ import annotations

from typing import Any

from control_plane.streamlit_util import (
    api_get_json,
    get_api_base,
    get_grafana_url,
    get_questdb_console_url,
    operator_logout,
)
from control_plane.watchlist import get_watchlist_symbols


def _active_watching_symbols(st_data: dict[str, Any]) -> list[str]:
    """Symbols with effective lifecycle ``active`` from ``GET /status`` → ``asset_lifecycle.states`` (FB-AP-033)."""
    lc = st_data.get("asset_lifecycle") or {}
    states = lc.get("states") or {}
    out = [sym for sym, st in states.items() if str(st).strip() == "active"]
    return sorted(out)


def render_app_sidebar() -> None:
    """Persistent sidebar: nav links, **Account** (gear), sign-in/out, status-derived links."""
    import streamlit as st

    st.sidebar.page_link("Home.py", label="Dashboard", icon=":material/dashboard:")
    st.sidebar.page_link("pages/Asset.py", label="Asset page", icon=":material/show_chart:")
    st.sidebar.page_link("pages/7_Account.py", label="Account", icon=":material/settings:")
    st.sidebar.page_link("pages/0_Login.py", label="Sign in / Register", icon=":material/login:")
    if st.session_state.get("operator_session_token"):
        if st.sidebar.button("Sign out"):
            operator_logout()
            st.rerun()
    try:
        st_data = api_get_json("/status")
        syms = st_data.get("symbols") or []
        if syms:
            st.sidebar.caption("Open asset (from config)")
            for s in syms[:24]:
                label = str(s).strip()
                if label:
                    st.sidebar.page_link(
                        "pages/Asset.py",
                        label=label,
                        query_params={"symbol": label},
                    )
        active_syms = _active_watching_symbols(st_data)
        st.sidebar.caption("Active (watching — **FB-AP-033**)")
        if active_syms:
            for s in active_syms:
                st.sidebar.page_link(
                    "pages/Asset.py",
                    label=f"● {s}",
                    query_params={"symbol": s},
                )
        else:
            st.sidebar.caption("No assets in **active** state.")
        wl = get_watchlist_symbols(st.session_state)
        st.sidebar.caption("Watchlist (session — **FB-UX-014**)")
        if wl:
            for s in wl:
                st.sidebar.page_link(
                    "pages/Asset.py",
                    label=f"★ {s}",
                    query_params={"symbol": s},
                )
        else:
            st.sidebar.caption("Pin symbols on **Asset**.")
        prof = st_data.get("execution_profile") or {}
        if prof.get("legacy_api_enabled") is False:
            active = prof.get("default_execution_mode", "?")
            mode_caption = (
                f"Default execution mode (env): `{active}` — per-asset overrides on **Asset** page "
                f"(**FB-AP-030**). Set **NM_EXECUTION_PROFILE_LEGACY_API=true** only for app-wide API."
            )
        else:
            active = prof.get("active_execution_mode", "?")
            mode_caption = f"Process execution mode (env): `{active}` — legacy profile API enabled."
    except Exception as e:
        st.sidebar.error(f"Cannot read status: {e}")
        active = "?"
        mode_caption = f"Process execution mode (env): `{active}` — change via `.env` / restart."
    st.sidebar.markdown(f"**Control plane:** `{get_api_base()}`")
    st.sidebar.markdown(f"**QuestDB:** `{get_questdb_console_url()}`")
    st.sidebar.markdown(f"**Grafana:** `{get_grafana_url()}`")

    st.sidebar.caption(mode_caption)
    st.sidebar.divider()
