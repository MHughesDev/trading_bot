"""Asset Page content (FB-AP-027 / FB-AP-028) — one canonical symbol; calls control plane read APIs."""

from __future__ import annotations

import logging
from typing import Any

import httpx
import streamlit as st

from control_plane.asset_chart_data import Preset, preset_to_request
from control_plane.chart_block import (
    build_ohlc_figure,
    render_chart_card,
    render_timeframe_selector,
)
from control_plane.asset_chart_fetch import (
    chart_time_window,
    fetch_bars_for_preset,
    fetch_trade_markers_for_window,
)
from control_plane.asset_chart_markers import trade_marker_buy_sell_traces
from control_plane.asset_lifecycle_actions import (
    lifecycle_action_spec,
    poll_init_job_until_terminal,
)
from control_plane.app_toasts import maybe_toast
from control_plane.asset_manifest_card import manifest_model_rows
from control_plane.asset_page_helpers import normalize_symbol, validate_symbol_display
from control_plane.manual_trade_form import (
    ORDER_TYPES,
    TIME_IN_FORCE,
    build_manual_order_body,
)
from control_plane.navigation import DASHBOARD_PAGE
from control_plane.watchlist import add_watchlist_symbol, is_pinned, remove_watchlist_symbol
from control_plane.streamlit_util import (
    api_delete_json,
    api_get_json,
    api_post_json,
    api_put_json,
    post_mutate_response,
)

logger = logging.getLogger(__name__)


def _run_initialize(sym: str, msg_key: str) -> None:
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


def _render_trade_panel(sym: str) -> None:
    """Manual buy/sell/flatten for a human operator (routes through /trade → risk + signing)."""
    st.markdown("#### Trade")
    st.caption("Manual order — same risk gate + signing as the automated bot (paper by default).")
    with st.form(f"trade_form_{sym}"):
        c1, c2, c3 = st.columns([1, 1, 1])
        side = c1.selectbox("Side", ["buy", "sell"], key=f"trade_side_{sym}")
        qty = c2.number_input(
            "Quantity", min_value=0.0, value=0.0, step=0.001, format="%.6f", key=f"trade_qty_{sym}"
        )
        otype = c3.selectbox("Type", list(ORDER_TYPES), key=f"trade_type_{sym}")
        p1, p2, p3 = st.columns([1, 1, 1])
        limit_price = p1.number_input(
            "Limit price", min_value=0.0, value=0.0, step=0.01, key=f"trade_limit_{sym}",
            help="Required for limit and stop-limit orders",
        )
        stop_price = p2.number_input(
            "Stop price", min_value=0.0, value=0.0, step=0.01, key=f"trade_stop_{sym}",
            help="Required for stop and stop-limit orders",
        )
        tif = p3.selectbox("Time in force", list(TIME_IN_FORCE), key=f"trade_tif_{sym}")
        submitted = st.form_submit_button("Place order", type="primary", use_container_width=True)
    if submitted:
        try:
            body: dict[str, Any] = build_manual_order_body(
                symbol=sym,
                side=side,
                quantity=qty,
                order_type=otype,
                limit_price=limit_price or None,
                stop_price=stop_price or None,
                time_in_force=tif,
            )
        except ValueError as e:
            st.error(str(e))
        else:
            try:
                r = api_post_json("/trade/order", body)
                if r.get("submitted"):
                    st.success(f"Order submitted: {side} {qty} {sym}")
                    maybe_toast(f"Order submitted: {side} {qty} {sym}", icon="✅")
                else:
                    reason = ", ".join(r.get("blocked") or []) or str(r.get("error") or "blocked")
                    st.warning(f"Order not submitted — {reason}.")
            except Exception as e:
                st.error(str(e))
    if st.button("Flatten position", key=f"trade_flatten_{sym}", use_container_width=True):
        try:
            r = api_post_json("/trade/flatten", {"symbol": sym})
            rep = r.get("flatten", {}) if isinstance(r, dict) else {}
            if rep.get("submitted"):
                st.success(f"Flatten submitted for {sym}.")
            else:
                st.info(f"Flatten: {rep.get('skipped') or rep.get('error') or 'no position'}.")
        except Exception as e:
            st.error(str(e))


def _render_ohlc_chart(sym: str) -> None:
    """FB-AP-028: OHLC from ``/assets/chart/bars``; week/month from daily rollup."""
    left, right = st.columns([4, 2])
    with left:
        st.markdown("**Asset price chart**")
    with right:
        labels = ["1s", "1m", "1h", "1D", "1W", "1M"]
        label_to_preset: dict[str, Preset] = {
            "1s": "second",
            "1m": "minute",
            "1h": "hour",
            "1D": "day",
            "1W": "week",
            "1M": "month",
        }
        choice_label = render_timeframe_selector(
            label="Interval",
            options=labels,
            state_key=f"asset_chart_interval_chip_{sym}",
            default="1m",
        )
    preset = label_to_preset[choice_label]
    req = preset_to_request(preset)
    try:
        from control_plane.streamlit_util import get_api_base

        api_base = get_api_base().rstrip("/")
        st.caption(
            f"**FB-AP-034** — SSE push (latest bar): `{api_base}/assets/chart/stream?symbol={sym}"
            f"&interval_seconds={req.interval_seconds}` (optional `poll_seconds`); "
            f"single snapshot: `GET {api_base}/assets/chart/latest-bar?symbol={sym}&interval_seconds=...`"
        )
    except Exception:
        logger.warning("asset_page: could not resolve API base for chart hints symbol=%s", sym, exc_info=True)
    try:
        bars, note, _eff = fetch_bars_for_preset(sym, preset)
    except httpx.HTTPStatusError as e:
        payload: dict[str, Any] = {}
        try:
            payload = e.response.json()
        except Exception:
            payload = {}
        code = e.response.status_code
        err = str(payload.get("error") or "").strip()
        detail = str(payload.get("detail") or e).strip()
        label = "Unexpected error" if code >= 500 else "Chart request failed"
        if err == "storage_unavailable" or code == 503:
            label = "Chart storage unavailable"
        st.markdown(
            (
                "<div style='border-left:4px solid var(--loss-red,#ef4444);"
                "padding:8px 10px;margin:8px 0;'>"
                f"<strong>{label}</strong>: {detail}"
                "</div>"
            ),
            unsafe_allow_html=True,
        )
        if st.button("Retry", key=f"asset_chart_retry_{sym}", use_container_width=False):
            st.rerun()
        return
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
        fig = build_ohlc_figure(
            bars=[],
            height=440,
            y_axis_title="Dollars",
            empty_annotation=f"No price history for {sym} yet.",
        )
        render_chart_card(
            title=f"{sym} price history",
            figure=fig,
            caption=f"{note}. Initialize this asset to begin filling the chart.",
            key=f"asset_ohlc_chart_empty_{sym}",
        )
        if st.button(
            "Initialize",
            type="primary",
            key=f"asset_chart_empty_init_{sym}",
            use_container_width=False,
        ):
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
                    maybe_toast(f"Init finished: {sym}", icon="✅")
                    st.success("Initialization finished.")
                else:
                    maybe_toast("Initialization failed.", icon="⚠️")
                    st.error(str(job.get("error") or "Initialization failed."))
            except Exception as exc:
                maybe_toast(str(exc)[:240], icon="⚠️")
                st.error(str(exc))
        return
    try:
        buy_tr, sell_tr = trade_marker_buy_sell_traces(markers, bars)
        fig = build_ohlc_figure(
            bars=bars,
            buy_markers=buy_tr,
            sell_markers=sell_tr,
            height=440,
            y_axis_title="Dollars",
        )
        render_chart_card(
            title=f"{sym} price history",
            figure=fig,
            caption=note,
            key=f"asset_ohlc_chart_{sym}",
        )
    except ImportError:
        st.caption("Install **plotly** (`pip install -e \".[dashboard]\"`) for candlesticks; showing close line.")
        st.line_chart(
            {"close": [float(b["close"]) for b in bars]},
            width="stretch",
        )


_BACKTEST_PRESETS: tuple[Preset, ...] = ("minute", "hour", "day", "week", "month")


def _backtest_param_input(sym: str, param: dict[str, Any]) -> Any:
    """Render one strategy-parameter widget from its descriptor; return the entered value."""
    name = param["name"]
    kind = param.get("kind", "string")
    default = param.get("default")
    help_text = param.get("description") or None
    key = f"asset_bt_param_{sym}_{name}"
    if kind == "int":
        return int(
            st.number_input(
                name,
                value=int(default if default is not None else 0),
                min_value=int(param["minimum"]) if param.get("minimum") is not None else None,
                max_value=int(param["maximum"]) if param.get("maximum") is not None else None,
                step=1,
                key=key,
                help=help_text,
            )
        )
    if kind == "float":
        return float(
            st.number_input(
                name,
                value=float(default if default is not None else 0.0),
                min_value=float(param["minimum"]) if param.get("minimum") is not None else None,
                max_value=float(param["maximum"]) if param.get("maximum") is not None else None,
                key=key,
                help=help_text,
            )
        )
    # decimal / string: keep as text so precision-sensitive values round-trip exactly.
    return st.text_input(name, value=str(default if default is not None else ""), key=key, help=help_text)


def _render_backtest_results(result: dict[str, Any]) -> None:
    """Render PnL stats + fills/positions from a /assets/backtest response."""
    st.caption(
        f"Ran **{result.get('strategy_key')}** on **{result.get('bar_count')}** bars "
        f"({result.get('start')} → {result.get('end')})."
    )
    pnls = result.get("stats_pnls") or {}
    for ccy, stats in pnls.items():
        cols = st.columns(4)
        cols[0].metric(f"PnL ({ccy})", f"{stats.get('PnL (total)', 0.0):,.2f}")
        cols[1].metric("PnL %", f"{stats.get('PnL% (total)', 0.0) * 100:,.3f}%")
        cols[2].metric("Win rate", f"{stats.get('Win Rate', 0.0) * 100:,.1f}%")
        cols[3].metric("Expectancy", f"{stats.get('Expectancy', 0.0):,.2f}")

    rcols = st.columns(3)
    rcols[0].metric("Orders", result.get("total_orders", 0))
    rcols[1].metric("Positions", result.get("total_positions", 0))
    rcols[2].metric("Iterations", result.get("iterations", 0))

    returns = result.get("stats_returns") or {}
    if returns:
        with st.expander("Returns stats", expanded=False):
            st.json(returns)

    fills = result.get("fills") or []
    if fills:
        st.markdown("**Order fills**")
        try:
            import polars as pl

            st.dataframe(pl.DataFrame(fills), hide_index=True, width="stretch")
        except Exception:
            st.table(fills)


def _render_backtest_panel(sym: str, lifecycle_state: str) -> None:
    """Strategy backtest — separate from paper/live, runnable concurrently (FB-AP-XXX).

    Gated on the asset being initialized (has data); requires a strategy from the dropdown.
    """
    st.markdown("#### Backtest")
    if lifecycle_state in ("uninitialized", "?"):
        st.caption("Initialize this asset (load its data) before backtesting.")
        return

    try:
        catalogue = api_get_json("/strategies")
    except Exception as e:
        st.warning(f"Strategies: {e}")
        return
    strategies = catalogue.get("strategies") or []
    if not strategies:
        st.caption("No strategies registered.")
        return

    by_key = {s["key"]: s for s in strategies}
    labels = {s["key"]: s.get("name") or s["key"] for s in strategies}
    keys = list(by_key)
    chosen_key = st.selectbox(
        "Strategy",
        options=keys,
        format_func=lambda k: labels.get(k, k),
        key=f"asset_bt_strategy_{sym}",
    )
    descriptor = by_key[chosen_key]
    if descriptor.get("description"):
        st.caption(descriptor["description"])

    preset = st.selectbox(
        "Window",
        options=_BACKTEST_PRESETS,
        index=1,  # default "hour"
        key=f"asset_bt_window_{sym}",
    )

    params: dict[str, Any] = {}
    descriptor_params = descriptor.get("params") or []
    if descriptor_params:
        with st.expander("Parameters", expanded=False):
            pcols = st.columns(min(3, len(descriptor_params)))
            for i, param in enumerate(descriptor_params):
                with pcols[i % len(pcols)]:
                    params[param["name"]] = _backtest_param_input(sym, param)

    if st.button("Run backtest", type="primary", key=f"asset_bt_run_{sym}"):
        start, end = chart_time_window(preset)
        body = {
            "strategy_key": chosen_key,
            "start": start.isoformat(),
            "end": end.isoformat(),
            "strategy_params": params,
            "interval_seconds": preset_to_request(preset).interval_seconds,
        }
        try:
            with st.spinner("Running backtest…"):
                result = api_post_json(f"/assets/backtest/{sym}", body, timeout=180.0)
            st.session_state[f"asset_bt_result_{sym}"] = result
        except httpx.HTTPStatusError as e:
            detail = e.response.text
            if e.response.status_code == 503:
                st.error(
                    "Backtest engine not installed. Install the platform's NautilusTrader fork: "
                    'pip install -e ".[backtest_nautilus]" (see strategies/README.md).'
                )
            else:
                st.error(f"Backtest failed ({e.response.status_code}): {detail}")
        except Exception as e:
            st.error(f"Backtest failed: {e}")

    result = st.session_state.get(f"asset_bt_result_{sym}")
    if isinstance(result, dict):
        st.divider()
        _render_backtest_results(result)


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

    action, action_label, action_color = lifecycle_action_spec(lifecycle_state)
    head1, head2, head3 = st.columns([5, 2, 2])
    with head1:
        st.page_link(DASHBOARD_PAGE, label="Dashboard", icon=":material/arrow_back:")
        st.markdown(f"### `{sym}`")
        badge_map = {
            "uninitialized": "UNINITIALIZED",
            "initialized_not_active": "READY",
            "active": "ACTIVE",
        }
        badge = badge_map.get(lifecycle_state, lifecycle_state.upper())
        st.markdown(f"<span style='border:1px solid #374151;border-radius:10px;padding:2px 10px;font-size:11px;'>{badge}</span>", unsafe_allow_html=True)
    with head2:
        st.caption("Watchlist")
        if is_pinned(st.session_state, sym):
            if st.button("★ Pinned", key=f"asset_wl_unpin_{sym}", use_container_width=True):
                remove_watchlist_symbol(st.session_state, sym)
                maybe_toast(f"Removed {sym} from watchlist.", icon="⭐")
                st.rerun()
        else:
            if st.button("☆ Pin", key=f"asset_wl_pin_{sym}", use_container_width=True):
                ok, wmsg = add_watchlist_symbol(st.session_state, sym)
                if ok:
                    maybe_toast(f"Pinned {sym} to watchlist.", icon="⭐")
                else:
                    maybe_toast(wmsg, icon="⚠️")
                st.rerun()
    with head3:
        st.caption("Action")
        if action == "initialize":
            st.markdown(f"<style>div[data-testid='stButton'] button[kind='primary']{{background:{action_color};}}</style>", unsafe_allow_html=True)
            if st.button(action_label, type="primary", key=f"asset_lc_init_{sym}", use_container_width=True):
                _run_initialize(sym, msg_key)
        elif action == "start":
            st.markdown(f"<style>div[data-testid='stButton'] button[kind='primary']{{background:{action_color};}}</style>", unsafe_allow_html=True)
            if st.button(action_label, type="primary", key=f"asset_lc_start_{sym}", use_container_width=True):
                try:
                    api_post_json(f"/assets/lifecycle/{sym}/start")
                    st.session_state[msg_key] = ("success", "Watch started (active).")
                    maybe_toast("Watch started (active).", icon="✅")
                except Exception as e:
                    st.session_state[msg_key] = ("error", str(e))
                st.rerun()
        elif action == "stop":
            st.markdown(f"<style>div[data-testid='stButton'] button[kind='primary']{{background:{action_color};}}</style>", unsafe_allow_html=True)
            if st.button(action_label, type="primary", key=f"asset_lc_stop_{sym}", use_container_width=True):
                try:
                    r = post_mutate_response(f"/assets/lifecycle/{sym}/stop", {})
                    if r.status_code == 200:
                        st.session_state[msg_key] = ("success", "Watch stopped.")
                    elif r.status_code == 502:
                        st.session_state[msg_key] = ("error", "Flatten failed — lifecycle still active.")
                    else:
                        r.raise_for_status()
                except Exception as e:
                    st.session_state[msg_key] = ("error", str(e))
                st.rerun()
        else:
            st.caption(f"`{lifecycle_state}`")

    st.markdown("#### Mode")
    try:
        em = api_get_json(f"/assets/execution-mode/{sym}")
        eff = str(em.get("execution_mode", "?"))
        default_em = str(em.get("default_execution_mode", "?"))
        if hasattr(st, "popover"):
            with st.popover(f"Mode · {eff}"):
                st.caption(f"Default `{default_em}`")
                choice = st.selectbox(
                    "Execution mode",
                    options=["(use default)", "paper", "live"],
                    index=(1 if em.get("override") == "paper" else 2 if em.get("override") == "live" else 0),
                    key=f"asset_exec_mode_{sym}",
                )
                if st.button("Save", key=f"asset_exec_apply_{sym}"):
                    try:
                        if choice == "(use default)":
                            try:
                                api_delete_json(f"/assets/execution-mode/{sym}")
                            except httpx.HTTPStatusError as e:
                                if e.response.status_code != 404:
                                    raise
                        else:
                            api_put_json(f"/assets/execution-mode/{sym}", {"execution_mode": choice})
                        st.rerun()
                    except Exception as e:
                        st.error(str(e))
        else:
            st.caption(f"Mode `{eff}` (default `{default_em}`)")
    except Exception as e:
        st.warning(f"Execution mode: {e}")

    st.markdown("#### Strategy")
    st.caption(
        "Pick a strategy and it *is* the live decision source for this asset — runs continuously "
        "(paper or live, per Mode above) the moment you save it. No separate enable step."
    )
    try:
        catalogue = api_get_json("/strategies")
        strategies = catalogue.get("strategies") or []
        by_key = {s["key"]: s for s in strategies}
        labels = {s["key"]: s.get("name") or s["key"] for s in strategies}
        sel = api_get_json(f"/assets/strategy/{sym}")
        eff_key = sel.get("strategy_key")
        if not strategies:
            st.caption("No strategies registered.")
        elif hasattr(st, "popover"):
            with st.popover(f"Strategy · {labels.get(eff_key, eff_key or '?')}"):
                st.caption(f"Default `{labels.get(sel.get('default_strategy_key'), sel.get('default_strategy_key'))}`")
                keys = ["(use default)"] + list(by_key)
                current = sel.get("override")
                choice = st.selectbox(
                    "Live strategy",
                    options=keys,
                    format_func=lambda k: "(use default)" if k == "(use default)" else labels.get(k, k),
                    index=(keys.index(current) if current in keys else 0),
                    key=f"asset_live_strategy_{sym}",
                )
                if choice != "(use default)" and by_key[choice].get("description"):
                    st.caption(by_key[choice]["description"])
                if st.button("Save", key=f"asset_live_strategy_apply_{sym}"):
                    try:
                        if choice == "(use default)":
                            try:
                                api_delete_json(f"/assets/strategy/{sym}")
                            except httpx.HTTPStatusError as e:
                                if e.response.status_code != 404:
                                    raise
                        else:
                            api_put_json(f"/assets/strategy/{sym}", {"strategy_key": choice})
                        st.rerun()
                    except Exception as e:
                        st.error(str(e))
        else:
            st.caption(f"Strategy `{labels.get(eff_key, eff_key or '?')}`")
    except Exception as e:
        st.warning(f"Strategy: {e}")

    _render_trade_panel(sym)

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
        st.markdown(
            (
                "<div class='tb-card' style='text-align:center;'>"
                f"<div style='color:#9CA3AF;'>No manifest for {sym} yet.</div>"
                "</div>"
            ),
            unsafe_allow_html=True,
        )
        if st.button("Initialize", type="primary", key=f"asset_manifest_empty_init_{sym}"):
            _run_initialize(sym, msg_key)

    st.divider()
    _render_ohlc_chart(sym)

    st.divider()
    _render_backtest_panel(sym, lifecycle_state)

    api_base = st.session_state.get("_cp_api_base", "")
    if not api_base:
        from control_plane.streamlit_util import get_api_base

        api_base = get_api_base()
        st.session_state["_cp_api_base"] = api_base

    st.divider()
    with st.expander("Developer / Debug", expanded=False):
        st.caption(
            f"SSE: `{api_base}/assets/chart/stream?symbol={sym}&interval_seconds={preset_to_request('minute').interval_seconds}`"
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
