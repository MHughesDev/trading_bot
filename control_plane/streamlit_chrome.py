"""Shared Streamlit sidebar chrome (FB-UX-018 curated navigation rail)."""

from __future__ import annotations

from decimal import Decimal, InvalidOperation
from typing import Any

from control_plane._theme import render_brand
from control_plane.streamlit_util import (
    api_get_json,
    operator_logout,
)


def _active_watching_symbols(st_data: dict[str, Any]) -> list[str]:
    """Symbols with effective lifecycle ``active`` from ``GET /status`` → ``asset_lifecycle.states`` (FB-AP-033)."""
    lc = st_data.get("asset_lifecycle") or {}
    states = lc.get("states") or {}
    out = [sym for sym, st in states.items() if str(st).strip() == "active"]
    return sorted(out)


def _position_rows_from_payload(payload: dict[str, Any]) -> list[dict[str, Any]]:
    """Return normalized non-zero position rows from ``/positions`` payload."""
    rows = payload.get("positions") or []
    out: list[dict[str, Any]] = []
    for row in rows:
        if not isinstance(row, dict):
            continue
        qty_s = str(row.get("quantity") or "0")
        try:
            qty = Decimal(qty_s)
        except (InvalidOperation, ValueError):
            continue
        if qty == 0:
            continue
        out.append(row)
    return out


def _fmt_signed_decimal(raw: Any, *, places: int = 2) -> str:
    try:
        d = Decimal(str(raw))
    except (InvalidOperation, ValueError):
        return str(raw)
    q = Decimal(10) ** -places
    return f"{d.quantize(q):+f}"


def _watching_suffix_from_status(st_data: dict[str, Any], symbol: str) -> str:
    """
    Optional ``last price / delta`` suffix for a watching row.

    ``/status`` does not guarantee a price cache today, so this returns ``""``
    when unavailable.
    """
    cache = st_data.get("price_cache")
    if not isinstance(cache, dict):
        return ""
    row = cache.get(symbol)
    if not isinstance(row, dict):
        return ""
    last = row.get("last_price")
    delta = row.get("delta_24h_pct")
    if last is None or delta is None:
        return ""
    return f" · {last} ({_fmt_signed_decimal(delta)}%)"


def render_app_sidebar() -> None:
    """Curated left rail: five primary links + Watching/Holdings expanders."""
    import streamlit as st

    render_brand()
    st.sidebar.divider()
    _home_page = st.session_state.get("_cp_home_page", "Home.py")
    st.sidebar.page_link(_home_page, label="Dashboard", icon=":material/dashboard:")
    st.sidebar.page_link("pages/Asset.py", label="Asset page", icon=":material/show_chart:")
    st.sidebar.page_link("pages/7_Account.py", label="Account", icon=":material/settings:")
    st.sidebar.page_link("pages/0_Login.py", label="Sign in", icon=":material/login:")
    st.sidebar.page_link("pages/99_Sign_up.py", label="Sign up", icon=":material/person_add:")
    if st.session_state.get("operator_session_token"):
        if st.sidebar.button("Sign out"):
            operator_logout()
            st.rerun()

    st.sidebar.divider()

    watch_key = "sidebar_expanded_watching"
    hold_key = "sidebar_expanded_holdings"
    if watch_key not in st.session_state:
        st.session_state[watch_key] = True
    if hold_key not in st.session_state:
        st.session_state[hold_key] = True

    try:
        st_data = api_get_json("/status")
        active_syms = _active_watching_symbols(st_data)
    except Exception as e:
        st.sidebar.error(f"Cannot read status: {e}")
        st_data = {}
        active_syms = []

    with st.sidebar.expander("Watching", expanded=bool(st.session_state.get(watch_key, True))):
        if active_syms:
            for sym in active_syms:
                st.page_link(
                    "pages/Asset.py",
                    label=f"{sym}{_watching_suffix_from_status(st_data, sym)}",
                    icon=":material/visibility:",
                    query_params={"symbol": sym},
                )
        else:
            st.caption("None")

    holdings_payload: dict[str, Any] = {}
    try:
        holdings_payload = api_get_json("/positions")
    except Exception:
        try:
            holdings_payload = api_get_json("/portfolio/positions")
        except Exception:
            holdings_payload = {}
    holdings = _position_rows_from_payload(holdings_payload)
    with st.sidebar.expander("Holdings", expanded=bool(st.session_state.get(hold_key, True))):
        if holdings:
            for row in holdings:
                sym = str(row.get("symbol") or "?")
                qty = str(row.get("quantity") or "0")
                upnl = row.get("unrealized_pnl")
                if upnl is None:
                    st.caption(f"{sym} · qty {qty}")
                    continue
                if str(upnl).strip().startswith("-"):
                    st.markdown(
                        f"<span style='color: var(--pnl-down);'>{sym} · qty {qty} · uPnL {upnl}</span>",
                        unsafe_allow_html=True,
                    )
                else:
                    st.markdown(
                        f"<span style='color: var(--pnl-up);'>{sym} · qty {qty} · uPnL {upnl}</span>",
                        unsafe_allow_html=True,
                    )
        else:
            st.caption("None")
    st.sidebar.divider()
