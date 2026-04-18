"""Create account — redirects to venue API key setup when required (FB-UX-015)."""

from __future__ import annotations

import streamlit as st

from control_plane._theme import inject_global_css, render_brand_lockup
from control_plane.streamlit_util import (
    get_api_base,
    operator_login,
    operator_register,
    redirect_after_session_login,
)

st.set_page_config(page_title="Sign up — Trading Bot", layout="centered", initial_sidebar_state="collapsed")
inject_global_css()
st.markdown("<div class='tb-auth-wrap'><div class='tb-auth-card'>", unsafe_allow_html=True)
render_brand_lockup(subtitle="Operate paper and live trading from one console.")
st.write("")
st.subheader("Create account")

with st.form("sign_up"):
    email = st.text_input("Email", autocomplete="email")
    pw = st.text_input("Password", type="password", autocomplete="new-password")
    pw2 = st.text_input("Confirm password", type="password", autocomplete="new-password")
    submitted = st.form_submit_button("Create account", type="primary", use_container_width=True)

if submitted:
    if pw != pw2:
        st.error("Passwords do not match.")
    elif len(pw) < 8:
        st.error("Password must be at least 8 characters.")
    else:
        ok, err = operator_register(email.strip(), pw)
        if not ok:
            st.error(err)
        else:
            ok2, err2 = operator_login(email.strip(), pw)
            if ok2:
                st.success("Account created — continue to API keys.")
                redirect_after_session_login()
            else:
                st.warning(
                    f"Account created but automatic sign-in failed: {err2}. "
                    "Use **Sign in** with your email and password."
                )

st.page_link("pages/0_Login.py", label="Already have an account? Sign in", icon=":material/login:")
st.caption("Paper trading only until live credentials are added.")
st.markdown("</div></div>", unsafe_allow_html=True)
st.caption(f"Control plane: `{get_api_base()}`")
