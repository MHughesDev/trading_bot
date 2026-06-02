"""Streamlit multipage app (spec §14). Run from repo root:

    streamlit run control_plane/Home.py

Pages live in `control_plane/pages/`. Set `NM_CONTROL_PLANE_URL` for API base.

**FB-AP-026:** Dashboard focuses on PnL + holdings; global system power and app-wide paper/live
controls were removed from this page (per-asset lifecycle and execution mode are tracked in
``docs/QUEUE_ARCHIVE.MD`` (queue system: ``docs/QUEUE_SCHEMA.md``) — FB-AP-005 / FB-AP-030 / FB-AP-039 / FB-AP-040).
"""

from __future__ import annotations

import streamlit as st

from control_plane.navigation import NAVIGATION_PAGES

st.set_page_config(
    page_title="Trading Bot",
    page_icon="◧",
    layout="wide",
    initial_sidebar_state="expanded",
)
if hasattr(st, "navigation") and hasattr(st, "Page"):
    nav = st.navigation(NAVIGATION_PAGES, position="hidden")
    nav.run()
else:
    from control_plane._theme import inject_global_css
    from control_plane.pnl_panel import render_pnl_panel
    from control_plane.streamlit_chrome import render_app_sidebar
    from control_plane.streamlit_util import require_streamlit_app_access

    require_streamlit_app_access()
    inject_global_css()
    st.title("Trading Bot")
    render_app_sidebar()
    render_pnl_panel()
