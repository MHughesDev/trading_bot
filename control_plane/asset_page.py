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
from control_plane.asset_lifecycle_actions import (
    poll_init_job_until_terminal,
    primary_lifecycle_action,
)
from control_plane.app_toasts import maybe_toast
from control_plane.asset_manifest_card import manifest_model_rows
from control_plane.asset_page_helpers import normalize_symbol, validate_symbol_display
from control_plane.streamlit_util import (
    api_delete_json,
    api_get_json,
    api_post_json,
    api_put_json,
    post_mutate_response,
)


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

    msg_key = f"asset_toast_{sym}"
    toast = st.session_state.pop(msg_key, None)
    if isinstance(toast, tuple) and len(toast) == 2:
        kind, text = toast
        if kind == "success":
            st.success(text)
        elif kind == "error":
            st.error(text)

    try:
        life = api_get_json(f"/assets/lifecycle/{sym}")
        lifecycle_state = str(life.get("lifecycle_state") or "uninitialized")
    except Exception as e:
        st.warning(f"Lifecycle: {e}")
        lifecycle_state = "?"

    h1, h2 = st.columns([4, 1])
    with h1:
        st.subheader(f"Asset · `{sym}`")
    with h2:
        st.caption("Control")
        action = primary_lifecycle_action(lifecycle_state) if lifecycle_state != "?" else None
        if action == "initialize":
            if st.button("Initialize", type="primary", key=f"asset_lc_init_{sym}", use_container_width=True):
                try:
                    with st.spinner("Running initialization pipeline…"):
                        r = api_post_json(f"/assets/init/{sym}")
                        jid = str(r.get("job_id") or "")
                        if jid:
                            st.session_state["init_monitor_job_id"] = jid
                        job = poll_init_job_until_terminal(
                            jid,
                            fetch_job=lambda j: api_get_json(f"/assets/init/jobs/{j}"),
                        )
                    if job.get("status") == "succeeded":
                        st.session_state[msg_key] = ("success", "Initialization finished.")
                        maybe_toast(f"Init finished: {sym}", icon="✅")
                    else:
                        st.session_state[msg_key] = (
                            "error",
                            str(job.get("error") or "Initialization failed."),
                        )
                        maybe_toast("Initialization failed.", icon="⚠️")
                except httpx.HTTPStatusError as e:
                    if e.response.status_code == 409:
                        st.session_state[msg_key] = (
                            "error",
                            "Another initialization job is already running.",
                        )
                        maybe_toast("Another init job is running.", icon="⚠️")
                    else:
                        st.session_state[msg_key] = ("error", str(e))
                        maybe_toast(str(e)[:240], icon="⚠️")
                except Exception as e:
                    st.session_state[msg_key] = ("error", str(e))
                    maybe_toast(str(e)[:240], icon="⚠️")
                st.rerun()
        elif action == "start":
            if st.button("Start", type="primary", key=f"asset_lc_start_{sym}", use_container_width=True):
                try:
                    api_post_json(f"/assets/lifecycle/{sym}/start")
                    st.session_state[msg_key] = ("success", "Watch started (active).")
                    maybe_toast("Watch started (active).", icon="✅")
                except Exception as e:
                    st.session_state[msg_key] = ("error", str(e))
                    maybe_toast(str(e)[:240], icon="⚠️")
                st.rerun()
        elif action == "stop":
            if st.button("Stop", type="primary", key=f"asset_lc_stop_{sym}", use_container_width=True):
                try:
                    r = post_mutate_response(f"/assets/lifecycle/{sym}/stop", {})
                    if r.status_code == 200:
                        data = r.json()
                        fr = data.get("flatten") if isinstance(data.get("flatten"), dict) else {}
                        skipped = fr.get("skipped")
                        submitted = fr.get("submitted")
                        if skipped == "flat":
                            msg = "Watch stopped — no open position to flatten."
                            maybe_toast("No position to flatten.", icon="ℹ️")
                        elif submitted:
                            msg = "Watch stopped — flatten order submitted."
                            maybe_toast("Flatten order submitted.", icon="✅")
                        else:
                            msg = "Watch stopped."
                            maybe_toast(msg, icon="✅")
                        st.session_state[msg_key] = ("success", msg)
                    elif r.status_code == 502:
                        try:
                            detail = r.json().get("detail", r.text)
                        except Exception:
                            detail = r.text
                        st.session_state[msg_key] = ("error", f"Stop failed (flatten): {detail}")
                        maybe_toast("Flatten failed — lifecycle still active.", icon="⚠️")
                    else:
                        r.raise_for_status()
                except Exception as e:
                    st.session_state[msg_key] = ("error", str(e))
                    maybe_toast(str(e)[:240], icon="⚠️")
                st.rerun()
        else:
            st.caption(f"`{lifecycle_state}`")

    st.caption(f"**Lifecycle:** `{lifecycle_state}`")

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
        st.markdown("**Model / manifest** (read-only)")
        st.caption(
            f"From **`GET /assets/models/{sym}`** — paths and **last trained** fields for this user when "
            "**`NM_MULTI_TENANT_DATA_SCOPING`** uses per-user registry (see docs)."
        )
        rows = manifest_model_rows(m)
        try:
            import polars as pl

            st.dataframe(
                pl.DataFrame(rows),
                hide_index=True,
                width="stretch",
            )
        except Exception:
            st.table(rows)
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
