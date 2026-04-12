"""Account — operator profile placeholder (FB-UX-005); venue keys land in FB-UX-006."""

from __future__ import annotations

import streamlit as st

from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import require_streamlit_app_access

require_streamlit_app_access()
render_app_sidebar()

st.title("Account")
st.caption("FB-UX-005 — settings entry point. Per-venue API keys: **FB-UX-006**.")

try:
    from control_plane.streamlit_util import api_get_json

    me = api_get_json("/auth/me")
    st.success(f"Signed in as **{me.get('email', '?')}** (user id `{me.get('id', '?')}`).")
except Exception:
    st.info(
        "Not signed in via session, or **`GET /auth/me`** failed. "
        "Use **Sign in / Register** with **`NM_AUTH_SESSION_ENABLED=true`** on the API."
    )

st.markdown(
    """
- **Kraken** market data: deployment **`NM_*`** keys only (not per-user) — see **`README.md`** / **`docs/READY_TO_RUN.MD`**.
- **Alpaca** / **Coinbase** execution keys will be managed here after **FB-UX-006**.
"""
)
