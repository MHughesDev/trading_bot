from __future__ import annotations

import os

import requests
import streamlit as st

API_BASE = os.getenv("NAUTILUS_API_BASE", "http://localhost:8000")

st.set_page_config(page_title="NautilusMonster Dashboard", layout="wide")
st.title("NautilusMonster V3 Dashboard")

tab_live, tab_regimes, tab_routes, tab_models, tab_logs, tab_emergency = st.tabs(
    ["Live", "Regimes", "Routes", "Models", "Logs", "Emergency"]
)


def _safe_get(path: str):
    try:
        resp = requests.get(f"{API_BASE}{path}", timeout=5)
        resp.raise_for_status()
        return resp.json()
    except Exception as exc:  # pragma: no cover
        return {"error": str(exc)}


with tab_live:
    st.subheader("System Status")
    st.json(_safe_get("/status"))

with tab_regimes:
    st.subheader("Recent Decision Traces (Regime/Forecast)")
    traces = _safe_get("/traces")
    st.json(traces)

with tab_routes:
    st.subheader("Recent Routes")
    st.json(_safe_get("/routes"))

with tab_models:
    st.subheader("Registered Models")
    st.json(_safe_get("/models"))

with tab_logs:
    st.subheader("Decision Logs")
    st.json(_safe_get("/traces"))

with tab_emergency:
    st.subheader("Emergency Controls")
    c1, c2, c3 = st.columns(3)
    with c1:
        if st.button("PAUSE_NEW_ENTRIES"):
            requests.post(f"{API_BASE}/system/mode", json={"mode": "PAUSE_NEW_ENTRIES"}, timeout=5)
            st.success("Requested PAUSE_NEW_ENTRIES")
    with c2:
        if st.button("REDUCE_ONLY"):
            requests.post(f"{API_BASE}/system/mode", json={"mode": "REDUCE_ONLY"}, timeout=5)
            st.success("Requested REDUCE_ONLY")
    with c3:
        if st.button("FLATTEN_ALL"):
            requests.post(f"{API_BASE}/flatten", timeout=5)
            st.warning("Requested FLATTEN_ALL")
