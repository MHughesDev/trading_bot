import os

import httpx
import streamlit as st

st.header("Models")
base = os.getenv("NM_CONTROL_PLANE_URL", "http://127.0.0.1:8000")
try:
    st.json(httpx.get(f"{base}/models", timeout=5.0).json())
except Exception as e:
    st.warning(str(e))
