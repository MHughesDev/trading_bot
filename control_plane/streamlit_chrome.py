"""Shared Streamlit sidebar chrome (FB-UX-005 — Account link + navigation)."""

from __future__ import annotations

from control_plane.streamlit_util import (
    api_get_json,
    get_api_base,
    get_grafana_url,
    get_questdb_console_url,
    operator_logout,
)


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
