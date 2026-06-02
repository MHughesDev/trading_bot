"""Create account — a guided multi-step sign-up flow (FB-UX-015).

Steps: email -> password -> Alpaca keys (with link + instructions) -> Webull keys
(with link + instructions). The account is created at the password step; venue keys are
optional and can be skipped (and changed later on the Account page). Replaces the old
single static form with a flow, while keeping the separate Sign-in screen.
"""

from __future__ import annotations

import streamlit as st

from control_plane._theme import inject_global_css, render_brand_lockup
from control_plane.streamlit_util import (
    api_put_json,
    get_api_base,
    operator_login,
    operator_register,
    redirect_authenticated_user_from_auth_page,
)

st.set_page_config(
    page_title="Sign up — Trading Bot", layout="centered", initial_sidebar_state="collapsed"
)

_STEP_KEY = "signup_step"
step = st.session_state.get(_STEP_KEY, "email")

# Only bounce already-authenticated users away before the flow starts. Once the account is
# created mid-flow (steps alpaca/webull) the user IS authenticated, so we must not redirect
# or they could never reach the venue-key steps.
if step == "email":
    redirect_authenticated_user_from_auth_page()

inject_global_css()
st.markdown("<div class='tb-auth-wrap'><div class='tb-auth-card'>", unsafe_allow_html=True)
render_brand_lockup(subtitle="Operate paper and live trading from one console.")
st.write("")

_STEPS = ["email", "password", "alpaca", "webull"]
_idx = _STEPS.index(step) if step in _STEPS else len(_STEPS) - 1
st.progress((_idx + 1) / len(_STEPS), text=f"Step {_idx + 1} of {len(_STEPS)}")


def _save_venue(body: dict) -> None:
    """Persist venue keys; tolerate per-user storage being disabled (503)."""
    if not body:
        return
    try:
        api_put_json("/auth/venue-credentials", body)
    except Exception as ex:  # 503 when master secret unset, etc. — never block signup.
        st.info(f"Keys not stored ({ex}). You can add them later under Account.")


if step == "email":
    st.subheader("Create account")
    with st.form("signup_form_email"):
        email = st.text_input("Email", autocomplete="email")
        nxt = st.form_submit_button("Continue", type="primary", use_container_width=True)
    if nxt:
        e = email.strip()
        if "@" not in e or "." not in e.split("@")[-1]:
            st.error("Enter a valid email address.")
        else:
            st.session_state["signup_email"] = e
            st.session_state[_STEP_KEY] = "password"
            st.rerun()
    st.page_link("pages/0_Login.py", label="Already have an account? Sign in", icon=":material/login:")

elif step == "password":
    st.subheader("Choose a password")
    st.caption(f"for **{st.session_state.get('signup_email', '')}**")
    with st.form("signup_password"):
        pw = st.text_input("Password", type="password", autocomplete="new-password")
        pw2 = st.text_input("Confirm password", type="password", autocomplete="new-password")
        nxt = st.form_submit_button("Create account", type="primary", use_container_width=True)
    if nxt:
        email = st.session_state.get("signup_email", "")
        if pw != pw2:
            st.error("Passwords do not match.")
        elif len(pw) < 8:
            st.error("Password must be at least 8 characters.")
        else:
            ok, err = operator_register(email, pw)
            if not ok:
                st.error(err)
            else:
                ok2, err2 = operator_login(email, pw)
                if ok2:
                    st.session_state[_STEP_KEY] = "alpaca"
                    st.rerun()
                else:
                    st.warning(f"Account created but sign-in failed: {err2}. Use **Sign in**.")

elif step == "alpaca":
    st.subheader("Connect Alpaca (paper trading)")
    st.markdown(
        "Alpaca powers **paper trading**. Create a free account and generate API keys, then "
        "paste them below. You can skip this and add it later under **Account**.\n\n"
        "1. Sign up at [alpaca.markets](https://alpaca.markets/)\n"
        "2. Open [your paper API keys](https://app.alpaca.markets/paper/dashboard/overview)\n"
        "3. Copy the **API Key ID** and **Secret Key**"
    )
    with st.form("signup_alpaca"):
        ak = st.text_input("Alpaca API key ID", type="password", autocomplete="off")
        asec = st.text_input("Alpaca API secret", type="password", autocomplete="off")
        c1, c2 = st.columns(2)
        cont = c1.form_submit_button("Save & continue", type="primary", use_container_width=True)
        skip = c2.form_submit_button("Skip for now", use_container_width=True)
    if cont or skip:
        if cont:
            body = {}
            if ak.strip():
                body["alpaca_api_key"] = ak.strip()
            if asec.strip():
                body["alpaca_api_secret"] = asec.strip()
            _save_venue(body)
        st.session_state[_STEP_KEY] = "webull"
        st.rerun()

elif step == "webull":
    st.subheader("Connect Webull")
    st.markdown(
        "Add **Webull** API keys to use your Webull account. You can skip this and add it "
        "later under **Account**.\n\n"
        "1. Sign up at [webull.com](https://www.webull.com/)\n"
        "2. Request **API access** and generate your **API key** and **secret**\n"
        "3. Paste them below"
    )
    with st.form("signup_webull"):
        wk = st.text_input("Webull API key", type="password", autocomplete="off")
        wsec = st.text_input("Webull API secret", type="password", autocomplete="off")
        c1, c2 = st.columns(2)
        done = c1.form_submit_button("Finish", type="primary", use_container_width=True)
        skip = c2.form_submit_button("Skip for now", use_container_width=True)
    if done or skip:
        if done:
            body = {}
            if wk.strip():
                body["webull_api_key"] = wk.strip()
            if wsec.strip():
                body["webull_api_secret"] = wsec.strip()
            _save_venue(body)
        # Clear flow state and enter the app.
        for k in ("signup_step", "signup_email"):
            st.session_state.pop(k, None)
        from control_plane.navigation import DASHBOARD_PAGE

        st.switch_page(DASHBOARD_PAGE)

st.markdown("</div></div>", unsafe_allow_html=True)
st.caption(f"Control plane: `{get_api_base()}`")
