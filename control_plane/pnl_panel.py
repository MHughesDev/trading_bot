"""Dashboard hero + watching/holdings panels (FB-UX-020)."""

from __future__ import annotations

from datetime import datetime
from decimal import Decimal
from typing import Any

def _fmt_money(raw: Any) -> str:
    try:
        v = Decimal(str(raw))
    except Exception:
        return str(raw)
    sign = "+" if v >= 0 else "-"
    return f"{sign}${abs(v):,.2f}"


def _fmt_qty(raw: Any) -> str:
    try:
        q = Decimal(str(raw))
    except Exception:
        return str(raw)
    return f"{q:,.6f}".rstrip("0").rstrip(".")


def _fmt_pct(raw: Any) -> str:
    try:
        p = Decimal(str(raw))
    except Exception:
        return str(raw)
    return f"{p:+.2f}%"


def _color_from_num(raw: Any) -> str:
    try:
        v = Decimal(str(raw))
    except Exception:
        return "#9CA3AF"
    return "var(--pnl-up)" if v >= 0 else "var(--pnl-down)"


def _bucket_seconds_for_range(range_key: str) -> int:
    return {
        "hour": 300,
        "day": 3600,
        "month": 86_400,
        "year": 86_400,
        "all": 86_400,
    }.get(range_key, 3600)


def _series_path(range_key: str, mode: str, bucket_seconds: int) -> str:
    return f"/pnl/series?range={range_key}&bucket_seconds={bucket_seconds}&mode={mode}"


def _active_watching_rows(status_payload: dict[str, Any]) -> list[dict[str, Any]]:
    states = ((status_payload.get("asset_lifecycle") or {}).get("states") or {})
    cache = status_payload.get("price_cache") if isinstance(status_payload.get("price_cache"), dict) else {}
    rows: list[dict[str, Any]] = []
    for sym, state in states.items():
        if str(state).strip() != "active":
            continue
        c = cache.get(sym) if isinstance(cache, dict) else None
        rows.append(
            {
                "symbol": sym,
                "last_price": c.get("last_price") if isinstance(c, dict) else None,
                "delta_24h_pct": c.get("delta_24h_pct") if isinstance(c, dict) else None,
            }
        )
    rows.sort(key=lambda r: str(r["symbol"]))
    return rows


def _holdings_rows(positions_payload: dict[str, Any]) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for row in positions_payload.get("positions") or []:
        if not isinstance(row, dict):
            continue
        try:
            qty = Decimal(str(row.get("quantity") or "0"))
        except Exception:
            continue
        if qty == 0:
            continue
        mark = row.get("mark_price")
        avg = row.get("avg_entry_price")
        mkt_val: str | None = None
        try:
            if mark is not None:
                mkt_val = str(qty * Decimal(str(mark)))
        except Exception:
            mkt_val = None
        out.append(
            {
                "symbol": row.get("symbol") or "?",
                "quantity": str(qty),
                "avg_entry_price": avg,
                "market_value": mkt_val,
                "unrealized_pnl": row.get("unrealized_pnl"),
            }
        )
    out.sort(key=lambda r: str(r["symbol"]))
    return out


def _render_hero_chart(series_payload: dict[str, Any]) -> None:
    import streamlit as st

    points = series_payload.get("points") or []
    if not points:
        st.caption("No realized P&L points in this window.")
        return
    xs: list[datetime] = []
    ys: list[float] = []
    for pt in points:
        try:
            xs.append(datetime.fromisoformat(str(pt.get("bucket_start", "")).replace("Z", "+00:00")))
            ys.append(float(Decimal(str(pt.get("cumulative_usd") or "0"))))
        except Exception:
            continue
    if not xs:
        st.caption("No plottable P&L points in this window.")
        return

    line_color = "#22D3A0" if ys[-1] - ys[0] >= 0 else "#F87171"
    try:
        import plotly.graph_objects as go

        fig = go.Figure(
            data=[
                go.Scatter(
                    x=xs,
                    y=ys,
                    mode="lines",
                    line={"color": line_color, "width": 2},
                    fill="tozeroy",
                    fillcolor="rgba(34,211,160,0.08)" if line_color == "#22D3A0" else "rgba(248,113,113,0.08)",
                    name="Cumulative P&L",
                )
            ]
        )
        fig.update_layout(
            template="plotly_dark",
            height=360,
            margin={"l": 10, "r": 10, "t": 8, "b": 8},
            paper_bgcolor="#0B0F17",
            plot_bgcolor="#0B0F17",
            xaxis={"showgrid": False},
            yaxis={"showgrid": True, "gridcolor": "#1F2937"},
        )
        st.plotly_chart(fig, width="stretch")
    except Exception:
        st.line_chart({"time": xs, "cumulative": ys}, x="time", y="cumulative", width="stretch")


def render_pnl_panel() -> None:
    """Dashboard body with mode toggle, hero chart, watching, and holdings."""
    import streamlit as st
    from control_plane.streamlit_util import api_get_json

    st.markdown("<div style='font-size:11px;letter-spacing:.08em;color:#6B7280;'>PORTFOLIO</div>", unsafe_allow_html=True)

    mode_key = "dashboard_pnl_mode"
    if mode_key not in st.session_state:
        st.session_state[mode_key] = "paper"

    top_left, top_right = st.columns([4, 1])
    with top_right:
        if hasattr(st, "segmented_control"):
            picked = st.segmented_control(
                "Mode",
                options=["paper", "live"],
                default=str(st.session_state[mode_key]),
                key="dashboard_mode_seg",
                label_visibility="collapsed",
            )
        else:
            picked = st.radio(
                "Mode",
                options=["paper", "live"],
                horizontal=True,
                index=0 if st.session_state[mode_key] == "paper" else 1,
                key="dashboard_mode_radio",
                label_visibility="collapsed",
            )
        st.session_state[mode_key] = str(picked or st.session_state[mode_key])

    range_labels = {
        "1D": "day",
        "1W": "day",
        "1M": "month",
        "3M": "month",
        "YTD": "year",
        "ALL": "all",
    }
    if "dashboard_range_label" not in st.session_state:
        st.session_state["dashboard_range_label"] = "1D"
    selected_label = st.radio(
        "Range",
        options=list(range_labels.keys()),
        horizontal=True,
        index=list(range_labels.keys()).index(str(st.session_state["dashboard_range_label"])),
        key="dashboard_range_radio",
        label_visibility="collapsed",
    )
    st.session_state["dashboard_range_label"] = selected_label
    range_key = range_labels[selected_label]

    mode = str(st.session_state[mode_key])
    bucket_seconds = _bucket_seconds_for_range(range_key)
    series_payload = api_get_json(_series_path(range_key, mode, bucket_seconds))
    summary = api_get_json(f"/pnl/summary?range={range_key}")

    realized = summary.get("realized_pnl_usd") or "0"
    unrealized = summary.get("unrealized_pnl_usd") or "0"
    hero_total = Decimal(str(realized)) + Decimal(str(unrealized))

    with top_left:
        st.markdown(
            f"<div style='font-size:56px;font-weight:600;line-height:1;'>"
            f"{_fmt_money(hero_total)}</div>",
            unsafe_allow_html=True,
        )
        st.markdown(
            f"<div style='font-size:14px;color:{_color_from_num(unrealized)};'>"
            f"Unrealized {_fmt_money(unrealized)} · Realized {_fmt_money(realized)}</div>",
            unsafe_allow_html=True,
        )

    _render_hero_chart(series_payload)

    status_payload: dict[str, Any] = {}
    try:
        status_payload = api_get_json("/status")
    except Exception:
        status_payload = {}
    positions_payload: dict[str, Any] = {}
    try:
        positions_payload = api_get_json("/positions")
    except Exception:
        positions_payload = {"positions": []}

    watching = _active_watching_rows(status_payload)
    holdings = _holdings_rows(positions_payload)

    left, right = st.columns([1, 1])
    with left:
        st.markdown(f"**WATCHING** ({len(watching)})")
        if not watching:
            st.caption("No active watchlist")
        for row in watching[:6]:
            sym = str(row["symbol"])
            price = row.get("last_price")
            delta = row.get("delta_24h_pct")
            right_text = f"{price}" if price is not None else "—"
            if delta is not None:
                right_text = f"{right_text} {_fmt_pct(delta)}"
            st.page_link("pages/Asset.py", label=f"{sym} · {right_text}", query_params={"symbol": sym})

    with right:
        st.markdown(f"**HOLDINGS** ({len(holdings)})")
        if not holdings:
            st.caption("No open positions")
        for row in holdings:
            upnl = row.get("unrealized_pnl")
            st.markdown(
                (
                    f"<div><strong>{row['symbol']}</strong> · qty {_fmt_qty(row['quantity'])} "
                    f"· avg {row.get('avg_entry_price') or '—'} · mkt {row.get('market_value') or '—'} "
                    f"· <span style='color:{_color_from_num(upnl)}'>uPnL {_fmt_money(upnl or 0)}</span></div>"
                ),
                unsafe_allow_html=True,
            )
