"""Asset Page — one canonical symbol (FB-AP-027).

Deep link example: ``http://localhost:8501/Asset?symbol=BTC-USD`` (path is ``/Asset`` from
``pages/Asset.py``).
"""

from __future__ import annotations

import streamlit as st
import streamlit.components.v1 as components

from charts import CHART_CONFIG, TradingChart
from charts.indicators import indicator_label_to_key, keys_from_labels
from charts.trading_chart import CHART_TYPES
from control_plane._theme import inject_global_css
from control_plane.asset_page import render_asset_page_or_pick
from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import require_streamlit_app_access

# Human-readable chart-type labels ↔ TradingChart.chart_type values.
_CHART_TYPE_LABELS = {"Candles": "candles", "Heikin-Ashi": "heikin_ashi", "Line": "line"}
_DEFAULT_INDICATORS = ["EMA (Exponential MA)", "Bollinger Bands", "VWAP", "RSI", "MACD"]

require_streamlit_app_access()
inject_global_css()
render_app_sidebar()
st.title("Asset")
render_asset_page_or_pick()

_raw_symbol = st.query_params.get("symbol")
if isinstance(_raw_symbol, list):
    _raw_symbol = _raw_symbol[0] if _raw_symbol else None

if _raw_symbol:
    st.divider()
    st.subheader("Technical Chart")
    c1, c2 = st.columns([1, 1])
    chart_type_label = c1.selectbox(
        "Chart type", list(_CHART_TYPE_LABELS), key="asset_chart_type"
    )
    timeframe = c2.selectbox(
        "Timeframe",
        options=CHART_CONFIG["topbar"]["timeframes"],
        index=CHART_CONFIG["topbar"]["timeframes"].index("1D"),
        key="asset_chart_timeframe",
    )
    label_to_key = indicator_label_to_key()
    selected_labels = st.multiselect(
        "Indicator lines (overlays draw on price; oscillators add panes below)",
        options=list(label_to_key),
        default=[lbl for lbl in _DEFAULT_INDICATORS if lbl in label_to_key],
        key="asset_chart_indicators",
    )
    chart_type = _CHART_TYPE_LABELS.get(chart_type_label, "candles")
    if chart_type not in CHART_TYPES:
        chart_type = "candles"
    try:
        chart = TradingChart(str(_raw_symbol), timeframe, chart_type=chart_type)
        for key in keys_from_labels(selected_labels):
            chart.add_indicator(key)
        components.html(chart.render_html(), height=CHART_CONFIG["size"]["height"])
    except ValueError as exc:
        st.warning(str(exc))
    except ModuleNotFoundError as exc:
        st.info(str(exc))
    except Exception as exc:
        st.error(f"Chart unavailable: {exc}")
