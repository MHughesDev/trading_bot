"""Streamlit sidebar: open positions from ``GET /portfolio/positions``."""

from __future__ import annotations

from datetime import timedelta
from decimal import Decimal, InvalidOperation

import streamlit as st

from control_plane.streamlit_util import api_get_json


def _pnl_html(value_str: str | None) -> str:
    if not value_str:
        return ""
    try:
        v = Decimal(value_str)
    except (InvalidOperation, ValueError):
        return f" · uPnL `{value_str}`"
    color = "#16a34a" if v >= 0 else "#dc2626"
    return f' · uPnL <span style="color:{color};font-weight:600;">{value_str}</span>'


def render_positions_sidebar() -> None:
    """Collapsible positions list with optional fixed-interval refresh."""
    with st.sidebar.expander("Open positions", expanded=False):
        st.caption("Venue positions via execution adapter (paper Alpaca / live Coinbase).")
        auto = st.checkbox("Auto-refresh every 30s", value=False, key="positions_auto")
        if st.button("Refresh now", key="positions_refresh_btn"):
            _fetch_and_show_positions()
        elif auto:
            _positions_auto_fragment()
        else:
            _fetch_and_show_positions()


@st.fragment(run_every=timedelta(seconds=30))
def _positions_auto_fragment() -> None:
    _fetch_and_show_positions()


def _fetch_and_show_positions() -> None:
    try:
        data = api_get_json("/portfolio/positions")
    except Exception as e:
        st.error(f"Failed to load positions: {e}")
        return
    if not data.get("ok"):
        err = data.get("error") or "unknown error"
        st.warning(f"Venue error: {err}")
        st.caption(f"Adapter: `{data.get('adapter', '?')}`")
        return
    rows = data.get("positions") or []
    pol = data.get("mark_price_policy") or {}
    src = pol.get("source", "?")
    st.caption(
        f"Adapter: `{data.get('adapter', '?')}` · mode: `{data.get('execution_mode', '?')}` "
        f"· mark: `{src}`"
    )
    if not rows:
        st.info("No open positions.")
        return
    for p in rows:
        sym = p.get("symbol", "?")
        qty = p.get("quantity", "0")
        avg = p.get("avg_entry_price")
        u_pnl = p.get("unrealized_pnl")
        mark = p.get("mark_price")
        msrc = p.get("mark_price_source")
        extra = ""
        if avg:
            extra += f" · avg `{avg}`"
        if mark:
            extra += f" · mark `{mark}`"
        if msrc:
            extra += f" (`{msrc}`)"
        extra += _pnl_html(u_pnl)
        st.markdown(f"**{sym}** · qty `{qty}`{extra}", unsafe_allow_html=True)
