"""Streamlit multipage app (spec §14). Run from repo root:

    streamlit run control_plane/Home.py

Pages live in `control_plane/pages/`. Set `NM_CONTROL_PLANE_URL` for API base.
"""

from __future__ import annotations

import os

import streamlit as st

st.set_page_config(page_title="NautilusMonster V3", layout="wide")
st.title("NautilusMonster V3")
api_base = os.getenv("NM_CONTROL_PLANE_URL", "http://127.0.0.1:8000")
st.sidebar.markdown(f"**Control plane:** `{api_base}`")
st.markdown(
    """
Use the sidebar to open **Live**, **Regimes**, **Routes**, **Models**, **Logs**, **Emergency**.

Wire panels to `/status`, QuestDB, and Loki as you harden the stack.
"""
)
