"""Dashboard: PnL chart + holdings (FB-AP-026)."""

from __future__ import annotations

from datetime import datetime
from decimal import Decimal, InvalidOperation
from typing import Any

import streamlit as st

from control_plane.csv_export import (
    csv_text_to_utf8_bytes,
    pnl_summary_to_csv_text,
    positions_payload_to_csv_text,
)
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


def _bucket_seconds_for_range(range_key: str) -> int:
    return {
        "hour": 300,
        "day": 3600,
        "month": 86400,
        "year": 86400,
        "all": 86400,
    }.get(range_key, 3600)


def _render_holdings_main(data: dict[str, Any] | None, *, fetch_error: str | None) -> None:
    st.subheader("Current holdings")
    st.caption("Open positions from the execution adapter (same source as the legacy portfolio API).")
    if fetch_error:
        st.error(f"Failed to load positions: {fetch_error}")
        return
    assert data is not None
    if not data.get("ok"):
        st.warning(data.get("error") or "positions unavailable")
        return
    rows = data.get("positions") or []
    if not rows:
        st.info("No open positions.")
        return
    table_rows = []
    for p in rows:
        table_rows.append(
            {
                "Symbol": p.get("symbol", "?"),
                "Qty": str(p.get("quantity", "")),
                "uPnL USD": p.get("unrealized_pnl") or "—",
                "Mark": p.get("mark_price") or "—",
            }
        )
    st.dataframe(table_rows, use_container_width=True, hide_index=True)


def render_pnl_panel() -> None:
    """Main dashboard: holdings + PnL timeframe + cumulative realized chart + unrealized summary."""
    positions_payload: dict[str, Any] | None = None
    positions_fetch_error: str | None = None
    try:
        positions_payload = api_get_json("/portfolio/positions")
    except Exception as e:
        positions_fetch_error = str(e)

    _render_holdings_main(positions_payload, fetch_error=positions_fetch_error)

    if positions_payload is not None:
        pos_csv = positions_payload_to_csv_text(positions_payload)
        st.download_button(
            label="Download positions (CSV)",
            data=csv_text_to_utf8_bytes(pos_csv),
            file_name="portfolio_positions.csv",
            mime="text/csv",
            key="dl_positions_csv_fb011",
            width="stretch",
        )

    st.divider()
    st.subheader("P&L")
    st.caption(
        "Realized chart sums the local ledger (`data/pnl_ledger.jsonl`) into time buckets; "
        "totals align with `GET /pnl/summary` for the selected window."
    )
    opts = ("hour", "day", "month", "year", "all")
    choice = st.selectbox("Timeframe", options=opts, index=1, key="pnl_range_select_fb026")

    bucket_sec = _bucket_seconds_for_range(choice)
    try:
        series_payload = api_get_json(f"/pnl/series?range={choice}&bucket_seconds={bucket_sec}")
    except Exception as e:
        st.error(f"Failed to load P&L series: {e}")
        series_payload = None

    try:
        summary = api_get_json(f"/pnl/summary?range={choice}")
    except Exception as e:
        st.error(f"Failed to load P&L summary: {e}")
        return

    sum_csv = pnl_summary_to_csv_text(summary)
    st.download_button(
        label="Download P&L summary (CSV)",
        data=csv_text_to_utf8_bytes(sum_csv),
        file_name=f"pnl_summary_{choice}.csv",
        mime="text/csv",
        key="dl_pnl_summary_csv_fb011",
        width="stretch",
    )

    r = summary.get("realized_pnl_usd")
    u = summary.get("unrealized_pnl_usd")
    w0 = summary.get("window_start") or "—"
    w1 = summary.get("window_end") or "—"
    st.caption(f"Window (UTC): `{w0}` → `{w1}` · range: `{summary.get('range', '?')}`")

    c1, c2 = st.columns(2)
    with c1:
        st.markdown(
            f"**Realized (window)** "
            f'<span style="color:{_color_for_pnl(r)};font-weight:700;">{_fmt_usd(r)}</span>',
            unsafe_allow_html=True,
        )
    with c2:
        if u is None:
            err = summary.get("positions_error") or "unknown"
            st.markdown("**Unrealized** —")
            st.warning(f"Could not sum positions: {err}")
        else:
            st.markdown(
                f"**Unrealized** "
                f'<span style="color:{_color_for_pnl(u)};font-weight:700;">{_fmt_usd(u)}</span>',
                unsafe_allow_html=True,
            )

    if series_payload and series_payload.get("points"):
        pts = series_payload["points"]
        times: list[datetime] = []
        cum_vals: list[float] = []
        for pt in pts:
            raw = pt.get("bucket_start")
            if not raw:
                continue
            try:
                t = datetime.fromisoformat(str(raw).replace("Z", "+00:00"))
            except ValueError:
                continue
            times.append(t)
            try:
                cum_vals.append(float(Decimal(str(pt.get("cumulative_usd", "0")))))
            except Exception:
                cum_vals.append(0.0)
        if times:
            st.caption("Cumulative realized P&L (USD) within the window")
            st.line_chart(
                {
                    "Time": times,
                    "Cumulative realized (USD)": cum_vals,
                },
                x="Time",
                y="Cumulative realized (USD)",
                width="stretch",
            )

    ledger = summary.get("ledger") or {}
    st.caption(f"Ledger: `{ledger.get('source_of_truth', '?')}` — {ledger.get('note', '')}")
