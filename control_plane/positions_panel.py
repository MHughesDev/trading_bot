"""Streamlit sidebar: open positions from ``GET /portfolio/positions``."""

from __future__ import annotations

from datetime import timedelta

import streamlit as st

from control_plane.streamlit_util import api_get_json


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
    st.caption(f"Adapter: `{data.get('adapter', '?')}` · mode: `{data.get('execution_mode', '?')}`")
    if not rows:
        st.info("No open positions.")
        return
    for p in rows:
        sym = p.get("symbol", "?")
        qty = p.get("quantity", "0")
        avg = p.get("avg_entry_price")
        u_pnl = p.get("unrealized_pnl")
        extra = ""
        if avg:
            extra += f" · avg `{avg}`"
        if u_pnl:
            extra += f" · uPnL `{u_pnl}`"
        st.markdown(f"**{sym}** · qty `{qty}`{extra}")
