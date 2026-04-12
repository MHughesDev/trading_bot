"""Sign in / sign up (FB-UX-003). Requires ``NM_AUTH_SESSION_ENABLED=true`` on the API for login."""

from __future__ import annotations

import streamlit as st

from control_plane.streamlit_util import get_api_base, operator_login, operator_register

st.set_page_config(page_title="Sign in — Trading Bot", layout="centered")
st.title("Sign in")

st.caption(f"Control plane: `{get_api_base()}` — enable **`NM_AUTH_SESSION_ENABLED=true`** for password login.")

with st.form("sign_in"):
    email = st.text_input("Email", autocomplete="email")
    password = st.text_input("Password", type="password", autocomplete="current-password")
    submitted = st.form_submit_button("Sign in", type="primary")

if submitted:
    ok, err = operator_login(email.strip(), password)
    if ok:
        st.success("Signed in — opening Dashboard.")
        st.switch_page("Home.py")
    else:
        st.error(err)

with st.expander("Create account"):
    with st.form("sign_up"):
        re = st.text_input("Email (register)", key="reg_email", autocomplete="email")
        rp = st.text_input("Password (register)", type="password", key="reg_pw", autocomplete="new-password")
        rp2 = st.text_input("Confirm password", type="password", key="reg_pw2", autocomplete="new-password")
        reg_sub = st.form_submit_button("Register")
    if reg_sub:
        if rp != rp2:
            st.error("Passwords do not match")
        else:
            ok, err = operator_register(re.strip(), rp)
            if ok:
                st.success("Account created — use **Sign in** above.")
            else:
                st.error(err)

st.page_link("Home.py", label="Back to Dashboard", icon=":material/dashboard:")
