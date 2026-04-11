"""Streamlit multipage app (spec §14). Run from repo root:

    streamlit run control_plane/Home.py

Pages live in `control_plane/pages/`. Set `NM_CONTROL_PLANE_URL` for API base.
"""

from __future__ import annotations

import streamlit as st

from control_plane.streamlit_util import (
    api_get_json,
    api_post_json,
    get_api_base,
    get_grafana_url,
    get_questdb_console_url,
)

st.set_page_config(page_title="NautilusMonster V3", layout="wide")
st.title("NautilusMonster V3")
st.sidebar.markdown(f"**Control plane:** `{get_api_base()}`")
st.sidebar.markdown(f"**QuestDB:** `{get_questdb_console_url()}`")
st.sidebar.markdown(f"**Grafana:** `{get_grafana_url()}`")

st.sidebar.divider()
st.sidebar.subheader("System power")
st.sidebar.caption(
    "OFF halts inference, trading, and offline training. "
    "When using run.bat, the background live runtime stops while API + dashboard stay up."
)
try:
    power_state = api_get_json("/system/power").get("power", "?")
except Exception as e:
    st.sidebar.error(f"Cannot read power: {e}")
    power_state = "?"

col_a, col_b = st.sidebar.columns(2)
with col_a:
    if st.button("ON", use_container_width=True, type="primary"):
        try:
            api_post_json("/system/power", {"power": "on"})
            st.success("Power set to ON")
            st.rerun()
        except Exception as e:
            st.error(str(e))
with col_b:
    if st.button("OFF", use_container_width=True):
        try:
            api_post_json("/system/power", {"power": "off"})
            st.warning("Power set to OFF (hard stop)")
            st.rerun()
        except Exception as e:
            st.error(str(e))

st.sidebar.markdown(f"**Current:** `{power_state}`")

st.markdown(
    """
Use the sidebar to open **Live**, **Regimes**, **Routes**, **Models**, **Logs**, **Emergency**.

Pages call the FastAPI control plane (`/status`, `/routes`, `/models`, `/flatten`) and link to observability URLs.

**System power** (sidebar): global ON/OFF persisted to `data/system_power.json`. OFF stops model inference and order submission in the decision path; offline training skips; the Kraken live loop exits when power is turned OFF.
"""
)
