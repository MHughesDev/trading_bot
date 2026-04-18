"""Sign in / sign up (FB-UX-003). Requires ``NM_AUTH_SESSION_ENABLED=true`` on the API for login."""

from __future__ import annotations

import streamlit as st

from control_plane._theme import inject_global_css, render_brand_lockup
from control_plane.streamlit_util import (
    get_api_base,
    operator_login,
    redirect_after_session_login,
)

st.set_page_config(page_title="Sign in — Trading Bot", layout="centered", initial_sidebar_state="collapsed")
inject_global_css()
st.markdown("<div class='tb-auth-wrap'><div class='tb-auth-card'>", unsafe_allow_html=True)
render_brand_lockup(subtitle="Operate paper and live trading from one console.")
st.write("")
st.subheader("Sign in")

with st.form("sign_in"):
    email = st.text_input("Email", autocomplete="email")
    password = st.text_input("Password", type="password", autocomplete="current-password")
    submitted = st.form_submit_button("Sign in", type="primary", use_container_width=True)

if submitted:
    ok, err = operator_login(email.strip(), password)
    if ok:
        st.success("Signed in.")
        redirect_after_session_login()
    else:
        st.error(err)

st.page_link("pages/99_Sign_up.py", label="Don't have an account? Create one", icon=":material/person_add:")
st.caption("Paper trading only until live credentials are added.")
st.markdown("</div></div>", unsafe_allow_html=True)
st.caption(f"Control plane: `{get_api_base()}`")
