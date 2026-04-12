"""Asset Page content (FB-AP-027) — one canonical symbol; calls control plane read APIs."""

from __future__ import annotations

import streamlit as st

from control_plane.asset_page_helpers import normalize_symbol, validate_symbol_display
from control_plane.streamlit_util import api_get_json


def render_asset_page(symbol: str) -> None:
    """Main column: lifecycle, manifest summary, deep links to chart APIs."""
    sym = normalize_symbol(symbol)
    st.subheader(f"Asset · `{sym}`")

    try:
        life = api_get_json(f"/assets/lifecycle/{sym}")
        st.markdown(f"**Lifecycle:** `{life.get('lifecycle_state', '?')}`")
    except Exception as e:
        st.warning(f"Lifecycle: {e}")

    try:
        m = api_get_json(f"/assets/models/{sym}")
        st.success("Per-asset manifest is present.")
        st.json({k: m.get(k) for k in ("canonical_symbol", "schema_version") if k in m})
    except Exception:
        st.info("No manifest for this symbol (uninitialized). Use **POST /assets/init/{symbol}** from the API or operator flow.")

    api_base = st.session_state.get("_cp_api_base", "")
    if not api_base:
        from control_plane.streamlit_util import get_api_base

        api_base = get_api_base()
        st.session_state["_cp_api_base"] = api_base

    st.markdown("**Chart data (read-only)**")
    st.caption(
        "OHLC: `GET /assets/chart/bars` · markers: `GET /assets/chart/trade-markers` "
        "(see `docs/PER_ASSET_OPERATOR.MD`)."
    )
    st.markdown(
        f"- Bars: `{api_base}/assets/chart/bars?symbol={sym}&start=<ISO>&end=<ISO>`\n"
        f"- Trade markers: `{api_base}/assets/chart/trade-markers?symbol={sym}&start=<ISO>&end=<ISO>`"
    )


def render_asset_page_or_pick() -> None:
    """If ``symbol`` query param set, show asset; else show picker."""
    qp = st.query_params
    raw = qp.get("symbol")
    if isinstance(raw, list):
        raw = raw[0] if raw else ""
    sym_in = normalize_symbol(str(raw or ""))

    if sym_in and validate_symbol_display(sym_in):
        render_asset_page(sym_in)
        return

    st.subheader("Asset page")
    st.caption("Open a symbol via the sidebar, or enter a canonical pair (e.g. **BTC-USD**).")
    c1, c2 = st.columns([3, 1])
    with c1:
        typed = st.text_input("Symbol", value=sym_in or "", key="asset_symbol_input", placeholder="BTC-USD")
    with c2:
        st.write("")
        st.write("")
        go = st.button("Open", type="primary", key="asset_open_btn")

    if go:
        t = normalize_symbol(typed)
        if not validate_symbol_display(t):
            st.error("Invalid symbol.")
            return
        st.query_params["symbol"] = t
        st.rerun()
