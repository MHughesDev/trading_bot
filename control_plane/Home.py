"""Streamlit multipage app (spec §14). Run from repo root:

    streamlit run control_plane/Home.py

Pages live in `control_plane/pages/`. Set `NM_CONTROL_PLANE_URL` for API base.

**FB-AP-026:** Dashboard focuses on PnL + holdings; global system power and app-wide paper/live
controls were removed from this page (per-asset lifecycle and execution mode are tracked in
``docs/QUEUE.MD`` — FB-AP-005 / FB-AP-030 / FB-AP-039 / FB-AP-040).
"""

from __future__ import annotations

import streamlit as st

from control_plane.pnl_panel import render_pnl_panel
from control_plane.positions_panel import render_positions_sidebar
from control_plane.streamlit_util import (
    api_get_json,
    get_api_base,
    get_grafana_url,
    get_questdb_console_url,
    operator_logout,
    require_streamlit_app_access,
)

st.set_page_config(page_title="Trading Bot", layout="wide")
require_streamlit_app_access()
st.title("Trading Bot")
st.sidebar.page_link("Home.py", label="Dashboard", icon=":material/dashboard:")
st.sidebar.page_link("pages/Asset.py", label="Asset page", icon=":material/show_chart:")
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

render_positions_sidebar()

render_pnl_panel()

st.markdown(
    """
Use the sidebar to open **Live**, **Regimes**, **Routes**, **Models**, **Logs**, **Emergency**.

Pages call the FastAPI control plane (`/status`, `/routes`, `/models`, `/flatten`) and link to observability URLs.

**Per-asset** controls (**Initialize / Start / Stop**, execution mode) are on the **Asset page** — see **`docs/PER_ASSET_OPERATOR.MD`** (**FB-AP-031**). System-wide power / hard stop removal: **FB-AP-039**.
"""
)
