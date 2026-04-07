import os

import httpx
import streamlit as st

st.header("Live")
base = os.getenv("NM_CONTROL_PLANE_URL", "http://127.0.0.1:8000")
try:
    r = httpx.get(f"{base}/status", timeout=5.0)
    st.json(r.json())
except Exception as e:
    st.warning(f"Could not reach control plane: {e}")
