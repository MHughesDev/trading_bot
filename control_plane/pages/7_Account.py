"""Account — operator profile and per-user venue API keys (FB-UX-005 / FB-UX-006)."""

from __future__ import annotations

import httpx
import streamlit as st

from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import (
    api_get_json,
    api_put_json,
    get_api_base,
    require_streamlit_app_access,
)

require_streamlit_app_access()
render_app_sidebar()

st.title("Account")
st.caption("Per-user **Alpaca** (paper) and **Coinbase** (live) keys — **FB-UX-006**. Kraken stays **`NM_*`** deployment-only.")

try:
    me = api_get_json("/auth/me")
    st.success(f"Signed in as **{me.get('email', '?')}** (user id `{me.get('id', '?')}`).")
except Exception:
    st.warning(
        "Sign in with **`NM_AUTH_SESSION_ENABLED=true`** on the API to manage venue keys. "
        "Set **`NM_STREAMLIT_ROUTE_GUARD_ENABLED=false`** or use **Sign in / Register** first."
    )
    st.stop()

st.markdown(
    "**Kraken** (market data): not stored here — configure **`NM_*`** in `.env` (see **`docs/operations/ready_to_run.md`**)."
)

try:
    vc = api_get_json("/auth/venue-credentials")
except httpx.HTTPStatusError as e:
    if e.response.status_code == 503:
        st.error(
            "Per-user venue storage is **off**: set **`NM_AUTH_VENUE_CREDENTIALS_MASTER_SECRET`** "
            "(long random string) in the control plane `.env` and restart **uvicorn**."
        )
        st.stop()
    raise

st.subheader("Alpaca (paper trading)")
c1, c2 = st.columns(2)
with c1:
    st.caption(
        f"Key: **{vc.get('alpaca_key_masked') or '—'}**"
        if vc.get("alpaca_key_set")
        else "Key: not set"
    )
with c2:
    st.caption(
        f"Secret: **{vc.get('alpaca_secret_masked') or '—'}**"
        if vc.get("alpaca_secret_set")
        else "Secret: not set"
    )

st.subheader("Coinbase Advanced Trade (live)")
c3, c4 = st.columns(2)
with c3:
    st.caption(
        f"Key: **{vc.get('coinbase_key_masked') or '—'}**"
        if vc.get("coinbase_key_set")
        else "Key: not set"
    )
with c4:
    st.caption(
        f"Secret: **{vc.get('coinbase_secret_masked') or '—'}**"
        if vc.get("coinbase_secret_set")
        else "Secret: not set"
    )

if vc.get("updated_at"):
    st.caption(f"Last updated: `{vc['updated_at']}`")

with st.form("venue_keys"):
    st.markdown("Enter new values only for fields you want to change (leave blank to keep current).")
    ak = st.text_input("Alpaca API key ID", type="password", autocomplete="off")
    asec = st.text_input("Alpaca API secret", type="password", autocomplete="off")
    ck = st.text_input("Coinbase API key", type="password", autocomplete="off")
    csec = st.text_input("Coinbase API secret", type="password", autocomplete="off")
    sub = st.form_submit_button("Save venue credentials")

if sub:
    body: dict = {}
    if ak.strip():
        body["alpaca_api_key"] = ak.strip()
    if asec.strip():
        body["alpaca_api_secret"] = asec.strip()
    if ck.strip():
        body["coinbase_api_key"] = ck.strip()
    if csec.strip():
        body["coinbase_api_secret"] = csec.strip()
    if not body:
        st.info("No new values entered.")
    else:
        try:
            api_put_json("/auth/venue-credentials", body)
            st.success("Saved. Full secrets are not shown again — only masked suffixes.")
            st.rerun()
        except Exception as ex:
            st.error(str(ex))

colx, coly = st.columns(2)
with colx:
    if st.button("Clear Alpaca keys"):
        try:
            api_put_json("/auth/venue-credentials", {"clear_alpaca": True})
            st.success("Alpaca credentials cleared.")
            st.rerun()
        except Exception as ex:
            st.error(str(ex))
with coly:
    if st.button("Clear Coinbase keys"):
        try:
            api_put_json("/auth/venue-credentials", {"clear_coinbase": True})
            st.success("Coinbase credentials cleared.")
            st.rerun()
        except Exception as ex:
            st.error(str(ex))

st.caption(f"API: `{get_api_base()}` — **`GET/PUT /auth/venue-credentials`**")
