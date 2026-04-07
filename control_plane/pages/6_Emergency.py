import os

import httpx
import streamlit as st

st.header("Emergency")
base = os.getenv("NM_CONTROL_PLANE_URL", "http://127.0.0.1:8000")
key = os.getenv("NM_CONTROL_PLANE_API_KEY", "")
if st.button("Request FLATTEN (calls POST /flatten)"):
    headers = {"X-API-Key": key} if key else {}
    try:
        r = httpx.post(f"{base}/flatten", timeout=10.0, headers=headers)
        st.json(r.json())
    except Exception as e:
        st.error(str(e))
st.caption("Set `NM_CONTROL_PLANE_API_KEY` if the API requires it.")
