"""Streamlit multipage app (spec §14). Run from repo root:

    streamlit run control_plane/Home.py

Pages live in `control_plane/pages/`. Set `NM_CONTROL_PLANE_URL` for API base.

**FB-AP-026:** Dashboard focuses on PnL + holdings; global system power and app-wide paper/live
controls were removed from this page (per-asset lifecycle and execution mode are tracked in
``docs/QUEUE_ARCHIVE.MD`` (queue system: ``docs/QUEUE_SCHEMA.md``) — FB-AP-005 / FB-AP-030 / FB-AP-039 / FB-AP-040).
"""

from __future__ import annotations

import streamlit as st

from control_plane._theme import inject_global_css
from control_plane.pnl_panel import render_pnl_panel
from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import require_streamlit_app_access

st.set_page_config(
    page_title="Trading Bot",
    page_icon="◧",
    layout="wide",
    initial_sidebar_state="expanded",
)


def _render_dashboard() -> None:
    require_streamlit_app_access()
    inject_global_css()
    st.title("Trading Bot")
    render_app_sidebar()
    render_pnl_panel()


if hasattr(st, "navigation") and hasattr(st, "Page"):
    nav = st.navigation(
        [
            st.Page(_render_dashboard, title="Dashboard", icon=":material/dashboard:", default=True),
            st.Page("pages/Asset.py", title="Asset page", icon=":material/show_chart:"),
            st.Page("pages/7_Account.py", title="Account", icon=":material/settings:"),
            st.Page("pages/0_Login.py", title="Sign in", icon=":material/login:"),
            st.Page("pages/99_Sign_up.py", title="Sign up", icon=":material/person_add:"),
            # Hidden but still routable via direct URL / deep links.
            st.Page("pages/1_Live.py", title="Live"),
            st.Page("pages/2_Regimes.py", title="Regimes"),
            st.Page("pages/3_Routes.py", title="Routes"),
            st.Page("pages/4_Models.py", title="Models"),
            st.Page("pages/5_Logs.py", title="Logs"),
            st.Page("pages/6_Emergency.py", title="Emergency"),
            st.Page("pages/98_Setup_API_keys.py", title="Setup API keys"),
        ],
        position="hidden",
    )
    nav.run()
else:
    _render_dashboard()
