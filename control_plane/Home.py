"""Streamlit multipage app (spec §14). Run from repo root:

    streamlit run control_plane/Home.py

Pages live in `control_plane/pages/`. Set `NM_CONTROL_PLANE_URL` for API base.

**FB-AP-026:** Dashboard focuses on PnL + holdings; global system power and app-wide paper/live
controls were removed from this page (per-asset lifecycle and execution mode are tracked in
``docs/QUEUE.MD`` — FB-AP-005 / FB-AP-030 / FB-AP-039 / FB-AP-040).
"""

from __future__ import annotations

import streamlit as st

from control_plane.health_strip import render_dashboard_health_strip
from control_plane.init_monitor import render_init_pipeline_monitor
from control_plane.pnl_panel import render_pnl_panel
from control_plane.scheduler_panel import render_scheduler_panel
from control_plane.positions_panel import render_positions_sidebar
from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import require_streamlit_app_access

st.set_page_config(page_title="Trading Bot", layout="wide")
require_streamlit_app_access()
st.title("Trading Bot")
render_app_sidebar()

render_dashboard_health_strip()

render_init_pipeline_monitor()

render_scheduler_panel()

render_positions_sidebar()

render_pnl_panel()

st.markdown(
    """
Use the sidebar to open **Live**, **Regimes**, **Routes**, **Models**, **Logs**, **Emergency**.

Pages call the FastAPI control plane (`/status`, `/routes`, `/models`, `/flatten`) and link to observability URLs.

**Per-asset** controls (**Initialize / Start / Stop**, execution mode) are on the **Asset page** — see **`docs/PER_ASSET_OPERATOR.MD`** (**FB-AP-031**). System-wide power / hard stop removal: **FB-AP-039**.
"""
)
