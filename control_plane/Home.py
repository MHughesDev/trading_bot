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
)

st.set_page_config(page_title="Trading Bot", layout="wide")
st.title("Trading Bot")
st.sidebar.markdown(f"**Control plane:** `{get_api_base()}`")
st.sidebar.markdown(f"**QuestDB:** `{get_questdb_console_url()}`")
st.sidebar.markdown(f"**Grafana:** `{get_grafana_url()}`")

try:
    st_data = api_get_json("/status")
    active = (st_data.get("execution_profile") or {}).get("active_execution_mode", "?")
except Exception as e:
    st.sidebar.error(f"Cannot read status: {e}")
    active = "?"

st.sidebar.caption(f"Process execution mode (env): `{active}` — change via `.env` / restart.")
st.sidebar.divider()

render_positions_sidebar()

render_pnl_panel()

st.markdown(
    """
Use the sidebar to open **Live**, **Regimes**, **Routes**, **Models**, **Logs**, **Emergency**.

Pages call the FastAPI control plane (`/status`, `/routes`, `/models`, `/flatten`) and link to observability URLs.

**Per-asset** Initialize / Start / Stop and execution routing are **not** on this dashboard — see **`docs/PER_ASSET_OPERATOR.MD`** and the queue (**FB-AP-031**, **FB-AP-030**). System-wide power / hard stop removal: **FB-AP-039**.
"""
)
