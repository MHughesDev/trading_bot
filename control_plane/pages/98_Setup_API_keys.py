"""Required onboarding wizard for per-user venue keys (FB-UX-017)."""

from __future__ import annotations

import httpx
import streamlit as st

from control_plane._theme import inject_global_css
from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import (
    api_get_json,
    api_put_json,
    get_api_base,
    require_streamlit_session_only,
    streamlit_venue_keys_gate_enabled,
)

_MASK_PREFIX = "•••• "


def _prefill_mask(masked: str | None) -> str:
    """Display masked suffix in input fields so operators can keep existing values."""
    if not masked:
        return ""
    tail = str(masked).replace("*", "").strip()
    return f"{_MASK_PREFIX}{tail}" if tail else ""


def _resolve_input(raw: str, *, masked_existing: str | None) -> str | None:
    """Return new value when changed; None means leave existing value unchanged."""
    v = str(raw or "").strip()
    if not v:
        return None
    if masked_existing and v == _prefill_mask(masked_existing):
        return None
    return v


def _venue_status(vc: dict[str, object]) -> tuple[bool, bool]:
    alpaca_ok = bool(vc.get("alpaca_key_set")) and bool(vc.get("alpaca_secret_set"))
    coinbase_ok = bool(vc.get("coinbase_key_set")) and bool(vc.get("coinbase_secret_set"))
    return alpaca_ok, coinbase_ok


def _current_step(vc: dict[str, object]) -> str:
    alpaca_ok, coinbase_ok = _venue_status(vc)
    if alpaca_ok and coinbase_ok:
        return "done"
    if alpaca_ok and not coinbase_ok:
        return "coinbase"
    return "alpaca"


def _save_partial(payload: dict[str, str | None]) -> bool:
    body = {k: v for k, v in payload.items() if v is not None}
    if not body:
        return True
    api_put_json("/auth/venue-credentials", body, require_key=False)
    return True


st.set_page_config(page_title="Set up API keys — Trading Bot", layout="centered")
require_streamlit_session_only()
inject_global_css()
render_app_sidebar()

st.title("Set up venue API keys")
st.caption(
    "Paper trading uses **Alpaca**; live trading uses **Coinbase Advanced Trade**. "
    "Kraken is used for **public** market data only — configure deployment env vars separately."
)

if not streamlit_venue_keys_gate_enabled():
    st.info("Venue key onboarding is **off** (`NM_STREAMLIT_VENUE_KEYS_REQUIRED` not set). Opening Dashboard.")
    st.switch_page("Home.py")
    st.stop()

try:
    me = api_get_json("/auth/me")
    st.success(f"Signed in as **{me.get('email', '?')}**.")
except Exception:
    st.error("Session invalid — sign in again.")
    st.switch_page("pages/0_Login.py")
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

step = st.session_state.get("venue_setup_step")
if step not in {"alpaca", "coinbase", "done"}:
    step = _current_step(vc)
    st.session_state["venue_setup_step"] = step

if _current_step(vc) == "done" or step == "done":
    st.session_state["venue_setup_step"] = "done"
    st.switch_page("Home.py")
    st.stop()

if step == "alpaca":
    st.progress(0.5)
    st.caption("Step 1 of 2")
    st.subheader("Alpaca (paper trading)")
    st.caption("Enter Alpaca API key + secret, then continue.")

    ak = st.text_input(
        "Alpaca API key ID",
        value=_prefill_mask(vc.get("alpaca_key_masked")),
        type="password",
        autocomplete="off",
        key="venue_alpaca_key",
    )
    asec = st.text_input(
        "Alpaca API secret",
        value=_prefill_mask(vc.get("alpaca_secret_masked")),
        type="password",
        autocomplete="off",
        key="venue_alpaca_secret",
    )

    c1, c2 = st.columns([1, 1])
    next_clicked = c1.button("Next", type="primary", use_container_width=True)
    skip_clicked = c2.button(
        "Skip (paper trading will be unavailable)",
        use_container_width=True,
    )

    if next_clicked:
        payload = {
            "alpaca_api_key": _resolve_input(ak, masked_existing=vc.get("alpaca_key_masked")),
            "alpaca_api_secret": _resolve_input(asec, masked_existing=vc.get("alpaca_secret_masked")),
        }
        alpaca_ok = bool(vc.get("alpaca_key_set")) and bool(vc.get("alpaca_secret_set"))
        if not alpaca_ok and (payload["alpaca_api_key"] is None or payload["alpaca_api_secret"] is None):
            st.error("Enter both Alpaca key ID and secret (or keep masked existing values).")
            st.stop()
        try:
            _save_partial(payload)
            st.session_state["venue_setup_step"] = "coinbase"
            st.rerun()
        except Exception as ex:
            st.error(str(ex))

    if skip_clicked:
        st.session_state["venue_setup_step"] = "coinbase"
        st.rerun()

elif step == "coinbase":
    st.progress(1.0)
    st.caption("Step 2 of 2")
    st.subheader("Coinbase Advanced Trade (live)")
    st.caption("Enter Coinbase key + secret, then continue.")

    ck = st.text_input(
        "Coinbase API key",
        value=_prefill_mask(vc.get("coinbase_key_masked")),
        type="password",
        autocomplete="off",
        key="venue_coinbase_key",
    )
    csec = st.text_input(
        "Coinbase API secret",
        value=_prefill_mask(vc.get("coinbase_secret_masked")),
        type="password",
        autocomplete="off",
        key="venue_coinbase_secret",
    )

    c1, c2 = st.columns([1, 1])
    next_clicked = c1.button("Next", type="primary", use_container_width=True)
    skip_clicked = c2.button(
        "Skip (live trading will be unavailable)",
        use_container_width=True,
    )

    if next_clicked:
        payload = {
            "coinbase_api_key": _resolve_input(ck, masked_existing=vc.get("coinbase_key_masked")),
            "coinbase_api_secret": _resolve_input(csec, masked_existing=vc.get("coinbase_secret_masked")),
        }
        coinbase_ok = bool(vc.get("coinbase_key_set")) and bool(vc.get("coinbase_secret_set"))
        if not coinbase_ok and (payload["coinbase_api_key"] is None or payload["coinbase_api_secret"] is None):
            st.error("Enter both Coinbase key and secret (or keep masked existing values).")
            st.stop()
        try:
            _save_partial(payload)
            st.session_state["venue_setup_step"] = "done"
            st.rerun()
        except Exception as ex:
            st.error(str(ex))

    if skip_clicked:
        st.session_state["venue_setup_step"] = "done"
        st.rerun()

st.caption(f"API: `{get_api_base()}`")
