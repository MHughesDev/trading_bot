"""Asset Page content (FB-AP-027 / FB-AP-028) — one canonical symbol; calls control plane read APIs."""

from __future__ import annotations

from typing import Any

import httpx
import streamlit as st

from control_plane.asset_chart_data import Preset
from control_plane.asset_chart_fetch import (
    chart_time_window,
    fetch_bars_for_preset,
    fetch_trade_markers_for_window,
)
from control_plane.asset_chart_markers import trade_marker_buy_sell_traces
from control_plane.asset_page_helpers import normalize_symbol, validate_symbol_display
from control_plane.streamlit_util import api_delete_json, api_get_json, api_put_json


def _render_ohlc_chart(sym: str) -> None:
    """FB-AP-028: OHLC from ``/assets/chart/bars``; week/month from daily rollup."""
    st.markdown("**OHLC chart**")
    preset_labels: dict[str, Preset] = {
        "second (1s)": "second",
        "minute (1m)": "minute",
        "hour (1h)": "hour",
        "day": "day",
        "week (from daily)": "week",
        "month (from daily)": "month",
    }
    choice = st.selectbox(
        "Interval",
        options=list(preset_labels.keys()),
        index=1,
        key=f"asset_chart_interval_{sym}",
    )
    preset = preset_labels[choice]
    try:
        bars, note, _eff = fetch_bars_for_preset(sym, preset)
    except Exception as e:
        st.error(f"Chart data failed: {e}")
        return
    st.caption(note)
    t_start, t_end = chart_time_window(preset)
    markers: list[dict[str, Any]] = []
    try:
        markers = fetch_trade_markers_for_window(sym, t_start, t_end)
    except Exception as e:
        st.caption(f"Trade markers unavailable: {e}")
    if markers:
        st.caption(f"Trade markers in window: **{len(markers)}** (source: `data/trade_markers.jsonl`)")
    if not bars:
        st.info("No bars returned — persist canonical bars to QuestDB or run startup backfill (see docs).")
        return
    try:
        import plotly.graph_objects as go

        buy_tr, sell_tr = trade_marker_buy_sell_traces(markers, bars)
        fig = go.Figure(
            data=[
                go.Candlestick(
                    x=[b["ts"] for b in bars],
                    open=[b["open"] for b in bars],
                    high=[b["high"] for b in bars],
                    low=[b["low"] for b in bars],
                    close=[b["close"] for b in bars],
                    name="OHLC",
                )
            ]
        )
        if buy_tr["x"]:
            fig.add_trace(
                go.Scatter(
                    x=buy_tr["x"],
                    y=buy_tr["y"],
                    mode="markers",
                    name="Buy",
                    marker=dict(size=11, color="#16a34a", symbol="triangle-up", line=dict(width=1, color="#14532d")),
                    text=buy_tr["text"],
                    hovertemplate="%{text}<extra></extra>",
                )
            )
        if sell_tr["x"]:
            fig.add_trace(
                go.Scatter(
                    x=sell_tr["x"],
                    y=sell_tr["y"],
                    mode="markers",
                    name="Sell",
                    marker=dict(size=11, color="#dc2626", symbol="triangle-down", line=dict(width=1, color="#7f1d1d")),
                    text=sell_tr["text"],
                    hovertemplate="%{text}<extra></extra>",
                )
            )
        fig.update_layout(
            xaxis_rangeslider_visible=False,
            height=440,
            margin=dict(l=10, r=10, t=30, b=10),
            legend=dict(orientation="h", yanchor="bottom", y=1.02, xanchor="right", x=1),
        )
        st.plotly_chart(fig, width="stretch")
    except ImportError:
        st.caption("Install **plotly** (`pip install -e \".[dashboard]\"`) for candlesticks; showing close line.")
        st.line_chart(
            {"close": [float(b["close"]) for b in bars]},
            width="stretch",
        )


def render_asset_page(symbol: str) -> None:
    """Main column: lifecycle, manifest summary, OHLC chart, API hints."""
    sym = normalize_symbol(symbol)
    st.subheader(f"Asset · `{sym}`")

    try:
        life = api_get_json(f"/assets/lifecycle/{sym}")
        st.markdown(f"**Lifecycle:** `{life.get('lifecycle_state', '?')}`")
    except Exception as e:
        st.warning(f"Lifecycle: {e}")

    st.markdown("**Execution mode (this symbol)**")
    try:
        em = api_get_json(f"/assets/execution-mode/{sym}")
        eff = str(em.get("execution_mode", "?"))
        default_em = str(em.get("default_execution_mode", "?"))
        st.caption(f"Application default: `{default_em}` · Effective for orders: **`{eff}`**")
        choice = st.selectbox(
            "Paper vs live (persisted)",
            options=["(use default)", "paper", "live"],
            index=(
                1
                if em.get("override") == "paper"
                else 2
                if em.get("override") == "live"
                else 0
            ),
            key=f"asset_exec_mode_{sym}",
        )
        c1, c2 = st.columns(2)
        with c1:
            if st.button("Apply execution mode", key=f"asset_exec_apply_{sym}"):
                try:
                    if choice == "(use default)":
                        try:
                            api_delete_json(f"/assets/execution-mode/{sym}")
                        except httpx.HTTPStatusError as e:
                            if e.response.status_code != 404:
                                raise
                    else:
                        api_put_json(
                            f"/assets/execution-mode/{sym}",
                            {"execution_mode": choice},
                        )
                    st.success("Updated.")
                    st.rerun()
                except Exception as e:
                    st.error(str(e))
        with c2:
            st.caption("Requires **NM_CONTROL_PLANE_API_KEY** when the API enforces it.")
    except Exception as e:
        st.warning(f"Execution mode: {e}")

    try:
        m = api_get_json(f"/assets/models/{sym}")
        st.success("Per-asset manifest is present.")
        st.json({k: m.get(k) for k in ("canonical_symbol", "schema_version") if k in m})
    except Exception:
        st.info("No manifest for this symbol (uninitialized). Use **POST /assets/init/{symbol}** from the API or operator flow.")

    st.divider()
    _render_ohlc_chart(sym)

    api_base = st.session_state.get("_cp_api_base", "")
    if not api_base:
        from control_plane.streamlit_util import get_api_base

        api_base = get_api_base()
        st.session_state["_cp_api_base"] = api_base

    st.divider()
    st.markdown("**Chart API (read-only)**")
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
