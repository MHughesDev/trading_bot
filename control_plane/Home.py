"""Streamlit multipage app (spec §14). Run from repo root:

    streamlit run control_plane/Home.py

Pages live in `control_plane/pages/`. Set `NM_CONTROL_PLANE_URL` for API base.
"""

from __future__ import annotations

import streamlit as st

from control_plane.streamlit_util import get_api_base, get_grafana_url, get_questdb_console_url

st.set_page_config(page_title="NautilusMonster V3", layout="wide")
st.title("NautilusMonster V3")
st.sidebar.markdown(f"**Control plane:** `{get_api_base()}`")
st.sidebar.markdown(f"**QuestDB:** `{get_questdb_console_url()}`")
st.sidebar.markdown(f"**Grafana:** `{get_grafana_url()}`")
st.markdown(
    """
Use the sidebar to open **Live**, **Regimes**, **Routes**, **Models**, **Logs**, **Emergency**.

Pages call the FastAPI control plane (`/status`, `/routes`, `/models`, `/flatten`) and link to observability URLs.
"""
)
