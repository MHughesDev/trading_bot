"""Dashboard page for Streamlit navigation."""

from __future__ import annotations

import streamlit as st
import streamlit.components.v1 as components

from charts import CHART_CONFIG, TradingChart
from control_plane._theme import inject_global_css
from control_plane.pnl_panel import render_pnl_panel
from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import require_streamlit_app_access

require_streamlit_app_access()
inject_global_css()
st.title("Trading Bot")
render_app_sidebar()
render_pnl_panel()

st.divider()
st.subheader("Reusable Trading Chart")
chart_cols = st.columns([2, 1])
with chart_cols[0]:
    dashboard_symbol = st.text_input("Symbol", value="BTC-USD", key="dashboard_chart_symbol")
with chart_cols[1]:
    dashboard_timeframe = st.selectbox(
        "Timeframe",
        options=CHART_CONFIG["topbar"]["timeframes"],
        index=CHART_CONFIG["topbar"]["timeframes"].index("1D"),
        key="dashboard_chart_timeframe",
    )

try:
    dashboard_chart = TradingChart(dashboard_symbol, dashboard_timeframe)
    dashboard_chart.add_indicator("sma", length=20).add_indicator("ema", length=50).add_indicator("rsi")
    dashboard_chart.add_indicator("macd").add_indicator("bollinger_bands", length=20, std_dev=2.0).add_indicator("vwap")
    components.html(dashboard_chart.render_html(), height=CHART_CONFIG["size"]["height"])
except ValueError as exc:
    st.warning(str(exc))
except ModuleNotFoundError as exc:
    st.info(str(exc))
except Exception as exc:
    st.error(f"Chart unavailable: {exc}")
