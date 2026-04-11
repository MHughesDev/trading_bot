"""Aggregate P&L from ``GET /pnl/summary`` (FB-DASH-05-03)."""

from __future__ import annotations

from decimal import Decimal, InvalidOperation

import streamlit as st

from control_plane.streamlit_util import api_get_json


def _fmt_usd(s: str | None) -> str:
    if s is None or s == "":
        return "—"
    try:
        v = Decimal(s)
    except (InvalidOperation, ValueError):
        return s
    sign = "" if v >= 0 else "-"
    abs_v = abs(v)
    return f"{sign}${abs_v:,.4f}"


def _color_for_pnl(s: str | None) -> str:
    if s is None or s == "":
        return "#6b7280"
    try:
        v = Decimal(s)
    except (InvalidOperation, ValueError):
        return "#6b7280"
    return "#16a34a" if v >= 0 else "#dc2626"


def render_pnl_panel() -> None:
    """Main-area panel: timeframe selector + realized / unrealized from API."""
    st.subheader("Aggregate P&L")
    st.caption(
        "Realized sums the local ledger (`data/pnl_ledger.jsonl`). "
        "Unrealized sums open-position uPnL from the execution adapter (same as Open positions)."
    )
    opts = ("hour", "day", "month", "year", "all")
    choice = st.selectbox("Timeframe", options=opts, index=1, key="pnl_range_select")
    try:
        data = api_get_json(f"/pnl/summary?range={choice}")
    except Exception as e:
        st.error(f"Failed to load P&L summary: {e}")
        return

    r = data.get("realized_pnl_usd")
    u = data.get("unrealized_pnl_usd")
    w0 = data.get("window_start") or "—"
    w1 = data.get("window_end") or "—"
    st.caption(f"Window (UTC): `{w0}` → `{w1}` · range: `{data.get('range', '?')}`")

    c1, c2 = st.columns(2)
    with c1:
        st.markdown(
            f"**Realized** "
            f'<span style="color:{_color_for_pnl(r)};font-weight:700;">{_fmt_usd(r)}</span>',
            unsafe_allow_html=True,
        )
    with c2:
        if u is None:
            err = data.get("positions_error") or "unknown"
            st.markdown("**Unrealized** —")
            st.warning(f"Could not sum positions: {err}")
        else:
            st.markdown(
                f"**Unrealized** "
                f'<span style="color:{_color_for_pnl(u)};font-weight:700;">{_fmt_usd(u)}</span>',
                unsafe_allow_html=True,
            )

    ledger = data.get("ledger") or {}
    st.caption(f"Ledger: `{ledger.get('source_of_truth', '?')}` — {ledger.get('note', '')}")
