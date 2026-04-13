"""Required onboarding: save Alpaca (paper) or Coinbase (live) API keys when gating is enabled (FB-UX-015)."""

from __future__ import annotations

import httpx
import streamlit as st

from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import (
    api_get_json,
    api_put_json,
    auth_me_venue_keys_complete,
    get_api_base,
    require_streamlit_session_only,
    streamlit_venue_keys_gate_enabled,
)

st.set_page_config(page_title="Set up API keys — Trading Bot", layout="centered")
require_streamlit_session_only()
render_app_sidebar()

st.title("Set up venue API keys")
st.caption(
    "Paper trading uses **Alpaca**; live trading uses **Coinbase Advanced Trade**. "
    "Kraken is used for **public** market data only — configure deployment env vars separately."
)

if not streamlit_venue_keys_gate_enabled():
    st.info("Venue key onboarding is **off** (`NM_STREAMLIT_VENUE_KEYS_REQUIRED` not set). Opening Dashboard.")
    st.switch_page("Home.py")

try:
    st_data = api_get_json("/status")
except Exception as e:
    st.error(f"Could not load **`/status`**: {e}")
    st.stop()

mode = str(st_data.get("execution_mode") or "paper")
st.markdown(f"Default execution mode from server: **`{mode}`** — you must provide keys for this mode.")

try:
    me = api_get_json("/auth/me")
    st.success(f"Signed in as **{me.get('email', '?')}**.")
except Exception:
    st.error("Session invalid — sign in again.")
    st.switch_page("pages/0_Login.py")
    st.stop()

if auth_me_venue_keys_complete() is True:
    st.success("Venue API keys are saved. Continuing to the Dashboard.")
    st.switch_page("Home.py")
    st.stop()

try:
    vc = api_get_json("/auth/venue-credentials")
except httpx.HTTPStatusError as e:
    if e.response.status_code == 503:
        st.error(
            "Per-user venue storage is **off**: set **`NM_AUTH_VENUE_CREDENTIALS_MASTER_SECRET`** "
            "in the API `.env` and restart **uvicorn**. You cannot complete this step until it is set."
        )
    else:
        st.error(str(e))
    st.stop()

if mode == "paper":
    st.subheader("Alpaca (paper trading)")
    st.caption("Create API keys in the Alpaca dashboard (paper trading enabled).")
else:
    st.subheader("Coinbase Advanced Trade (live)")
    st.caption("Use CDP API key + secret for Advanced Trade.")

with st.form("venue_onboarding"):
    if mode == "paper":
        ak = st.text_input("Alpaca API key ID", type="password", autocomplete="off")
        asec = st.text_input("Alpaca API secret", type="password", autocomplete="off")
        ck = ""
        csec = ""
    else:
        ak = ""
        asec = ""
        ck = st.text_input("Coinbase API key", type="password", autocomplete="off")
        csec = st.text_input("Coinbase API secret", type="password", autocomplete="off")
    sub = st.form_submit_button("Save and continue", type="primary")

if sub:
    body: dict = {}
    if mode == "paper":
        if not (ak.strip() and asec.strip()):
            st.error("Enter both Alpaca key ID and secret.")
        else:
            body["alpaca_api_key"] = ak.strip()
            body["alpaca_api_secret"] = asec.strip()
    else:
        if not (ck.strip() and csec.strip()):
            st.error("Enter both Coinbase key and secret.")
        else:
            body["coinbase_api_key"] = ck.strip()
            body["coinbase_api_secret"] = csec.strip()
    if body:
        try:
            api_put_json("/auth/venue-credentials", body)
            st.success("Saved. Opening Dashboard.")
            st.switch_page("Home.py")
        except Exception as ex:
            st.error(str(ex))

st.caption(f"API: `{get_api_base()}` · Change default paper/live in config or **Account** after onboarding.")
