"""Account — operator profile and per-user venue API keys (FB-UX-005 / FB-UX-006)."""

from __future__ import annotations

import httpx
import streamlit as st

from control_plane._theme import inject_global_css
from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import (
    api_get_json,
    api_put_json,
    get_api_base,
    require_streamlit_app_access,
)

require_streamlit_app_access()
inject_global_css()
render_app_sidebar()

st.title("Account")
st.caption("Per-user **Alpaca** (paper) and **Coinbase** (live) keys — **FB-UX-006**.")

try:
    me = api_get_json("/auth/me")
    st.success(f"Signed in as **{me.get('email', '?')}** (user id `{me.get('id', '?')}`).")
except Exception:
    st.warning(
        "Sign in with **`NM_AUTH_SESSION_ENABLED=true`** on the API to manage venue keys. "
        "Set **`NM_STREAMLIT_ROUTE_GUARD_ENABLED=false`** or use **Sign in / Register** first."
    )
    st.stop()

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

st.markdown("<div class='tb-card'>", unsafe_allow_html=True)
st.markdown("<div class='tb-section-eyebrow'>PROFILE</div>", unsafe_allow_html=True)
rows = [
    ("Email", me.get("email", "?")),
    ("Joined", str(me.get("created_at") or "—")),
    ("Last login", str(me.get("last_login_at") or "—")),
]
for label, value in rows:
    c1, c2 = st.columns([1, 2])
    with c1:
        st.caption(label)
    with c2:
        st.markdown(f"`{value}`")
st.markdown("</div>", unsafe_allow_html=True)

st.markdown("<div class='tb-card'>", unsafe_allow_html=True)
st.markdown("<div class='tb-section-eyebrow'>SECURITY</div>", unsafe_allow_html=True)
st.button("Sign out of all sessions", key="account_logout_all", use_container_width=True)
st.caption("Session history listing is not yet available in this build.")
st.markdown("</div>", unsafe_allow_html=True)

st.markdown("<div class='tb-card'>", unsafe_allow_html=True)
st.markdown("<div class='tb-section-eyebrow'>VENUE KEYS</div>", unsafe_allow_html=True)
st.markdown("**Alpaca**")
st.caption(
    f"Key: `{vc.get('alpaca_key_masked') or '—'}` · Secret: `{vc.get('alpaca_secret_masked') or '—'}` · "
    f"Status: {'Active' if vc.get('alpaca_key_set') and vc.get('alpaca_secret_set') else 'Not set'}"
)
st.markdown("**Coinbase**")
st.caption(
    f"Key: `{vc.get('coinbase_key_masked') or '—'}` · Secret: `{vc.get('coinbase_secret_masked') or '—'}` · "
    f"Status: {'Active' if vc.get('coinbase_key_set') and vc.get('coinbase_secret_set') else 'Not set'}"
)
st.page_link("pages/98_Setup_API_keys.py", label="Update venue keys", icon=":material/key:")

with st.form("venue_keys"):
    ak = st.text_input("Alpaca API key ID", type="password", autocomplete="off")
    asec = st.text_input("Alpaca API secret", type="password", autocomplete="off")
    ck = st.text_input("Coinbase API key", type="password", autocomplete="off")
    csec = st.text_input("Coinbase API secret", type="password", autocomplete="off")
    sub = st.form_submit_button("Save", use_container_width=True)

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
st.markdown("</div>", unsafe_allow_html=True)
st.caption(f"API: `{get_api_base()}`")
