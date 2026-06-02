"""Reusable Plotly chart blocks for the Streamlit control plane."""

from __future__ import annotations

from datetime import datetime
from decimal import Decimal
from typing import Any, Iterable


def render_timeframe_selector(
    *,
    label: str,
    options: list[str],
    state_key: str,
    default: str,
) -> str:
    """Render a shared timeframe switcher and persist the selected label."""
    import streamlit as st

    if state_key not in st.session_state:
        st.session_state[state_key] = default
    current = str(st.session_state.get(state_key) or default)
    if hasattr(st, "segmented_control"):
        selected = st.segmented_control(
            label,
            options=options,
            default=current if current in options else default,
            key=f"{state_key}__segmented",
            label_visibility="collapsed",
        )
    else:
        selected = st.radio(
            label,
            options=options,
            horizontal=True,
            index=options.index(current) if current in options else options.index(default),
            key=f"{state_key}__radio",
            label_visibility="collapsed",
        )
    out = str(selected or default)
    st.session_state[state_key] = out
    return out


def _base_figure(*, height: int, y_axis_title: str, annotation: str | None = None):
    import plotly.graph_objects as go

    fig = go.Figure()
    fig.update_layout(
        template="plotly_dark",
        height=height,
        margin={"l": 12, "r": 12, "t": 12, "b": 12},
        paper_bgcolor="#111827",
        plot_bgcolor="#111827",
        xaxis={
            "title": "Time",
            "showgrid": False,
            "gridcolor": "#1F2937",
        },
        yaxis={
            "title": y_axis_title,
            "showgrid": True,
            "gridcolor": "#1F2937",
            "zeroline": False,
        },
        legend={"orientation": "h", "yanchor": "bottom", "y": 1.02, "xanchor": "right", "x": 1},
    )
    if annotation:
        fig.add_annotation(
            text=annotation,
            x=0.5,
            y=0.5,
            xref="paper",
            yref="paper",
            showarrow=False,
            font={"color": "#9CA3AF", "size": 14},
        )
    return fig


def build_blank_figure(*, height: int, y_axis_title: str, annotation: str | None = None):
    """Blank chart container with axes and optional center annotation."""
    fig = _base_figure(height=height, y_axis_title=y_axis_title, annotation=annotation)
    fig.update_xaxes(showticklabels=False, zeroline=False)
    fig.update_yaxes(showticklabels=False)
    return fig


def build_line_figure(
    *,
    xs: Iterable[datetime],
    ys: Iterable[float],
    height: int = 360,
    y_axis_title: str = "Dollars",
    series_name: str = "Series",
    empty_annotation: str = "No data available for this window.",
):
    """Reusable time-series line chart with filled empty-state support."""
    import plotly.graph_objects as go

    xs_list = list(xs)
    ys_list = list(ys)
    if not xs_list or not ys_list:
        return build_blank_figure(height=height, y_axis_title=y_axis_title, annotation=empty_annotation)

    line_color = "#22D3A0" if ys_list[-1] - ys_list[0] >= 0 else "#F87171"
    fig = _base_figure(height=height, y_axis_title=y_axis_title)
    fig.add_trace(
        go.Scatter(
            x=xs_list,
            y=ys_list,
            mode="lines",
            line={"color": line_color, "width": 2},
            fill="tozeroy",
            fillcolor="rgba(34,211,160,0.08)" if line_color == "#22D3A0" else "rgba(248,113,113,0.08)",
            name=series_name,
        )
    )
    return fig


def build_ohlc_figure(
    *,
    bars: list[dict[str, Any]],
    buy_markers: dict[str, list[Any]] | None = None,
    sell_markers: dict[str, list[Any]] | None = None,
    height: int = 440,
    y_axis_title: str = "Dollars",
    empty_annotation: str = "No price history for this window.",
):
    """Reusable OHLC chart with optional buy/sell overlays and empty-state support."""
    import plotly.graph_objects as go

    if not bars:
        return build_blank_figure(height=height, y_axis_title=y_axis_title, annotation=empty_annotation)

    fig = _base_figure(height=height, y_axis_title=y_axis_title)
    fig.add_trace(
        go.Candlestick(
            x=[b["ts"] for b in bars],
            open=[b["open"] for b in bars],
            high=[b["high"] for b in bars],
            low=[b["low"] for b in bars],
            close=[b["close"] for b in bars],
            increasing_line_color="#22D3A0",
            decreasing_line_color="#F87171",
            increasing_fillcolor="#22D3A0",
            decreasing_fillcolor="#F87171",
            name="OHLC",
        )
    )
    fig.update_layout(xaxis_rangeslider_visible=False)
    if buy_markers and buy_markers.get("x"):
        fig.add_trace(
            go.Scatter(
                x=buy_markers["x"],
                y=buy_markers["y"],
                mode="markers",
                name="Buy",
                marker=dict(size=11, color="#16a34a", symbol="triangle-up", line=dict(width=1, color="#14532d")),
                text=buy_markers["text"],
                hovertemplate="%{text}<extra></extra>",
            )
        )
    if sell_markers and sell_markers.get("x"):
        fig.add_trace(
            go.Scatter(
                x=sell_markers["x"],
                y=sell_markers["y"],
                mode="markers",
                name="Sell",
                marker=dict(size=11, color="#dc2626", symbol="triangle-down", line=dict(width=1, color="#7f1d1d")),
                text=sell_markers["text"],
                hovertemplate="%{text}<extra></extra>",
            )
        )
    return fig


def render_chart_card(*, title: str, figure, caption: str | None = None, key: str) -> None:
    """Render a consistent chart section across pages."""
    import streamlit as st

    with st.container():
        st.markdown("<div class='tb-card'>", unsafe_allow_html=True)
        st.markdown(f"**{title}**")
        st.plotly_chart(figure, width="stretch", key=key)
        if caption:
            st.caption(caption)
        st.markdown("</div>", unsafe_allow_html=True)


def series_points_to_xy(points: list[dict[str, Any]]) -> tuple[list[datetime], list[float]]:
    """Normalize API point payloads into Plotly-ready X/Y arrays."""
    xs: list[datetime] = []
    ys: list[float] = []
    for pt in points:
        try:
            xs.append(datetime.fromisoformat(str(pt.get("bucket_start", "")).replace("Z", "+00:00")))
            ys.append(float(Decimal(str(pt.get("cumulative_usd") or "0"))))
        except Exception:
            continue
    return xs, ys
