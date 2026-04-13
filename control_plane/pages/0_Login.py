"""Sign in / sign up (FB-UX-003). Requires ``NM_AUTH_SESSION_ENABLED=true`` on the API for login."""

from __future__ import annotations

import streamlit as st

from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import (
    get_api_base,
    operator_login,
    redirect_after_session_login,
)

st.set_page_config(page_title="Sign in — Trading Bot", layout="centered")
render_app_sidebar()
st.title("Sign in")

st.page_link("pages/99_Sign_up.py", label="Create an account", icon=":material/person_add:")

st.caption(f"Control plane: `{get_api_base()}` — enable **`NM_AUTH_SESSION_ENABLED=true`** for password login.")

with st.form("sign_in"):
    email = st.text_input("Email", autocomplete="email")
    password = st.text_input("Password", type="password", autocomplete="current-password")
    submitted = st.form_submit_button("Sign in", type="primary")

if submitted:
    ok, err = operator_login(email.strip(), password)
    if ok:
        st.success("Signed in.")
        redirect_after_session_login()
    else:
        st.error(err)

st.page_link("Home.py", label="Back to Dashboard", icon=":material/dashboard:")
