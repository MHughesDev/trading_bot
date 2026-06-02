"""Asset Page — one canonical symbol (FB-AP-027).

Deep link example: ``http://localhost:8501/Asset?symbol=BTC-USD`` (path is ``/Asset`` from
``pages/Asset.py``).
"""

from __future__ import annotations

import streamlit as st
import streamlit.components.v1 as components

from charts import CHART_CONFIG, TradingChart
from control_plane._theme import inject_global_css
from control_plane.asset_page import render_asset_page_or_pick
from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import require_streamlit_app_access

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
    st.subheader("Reusable Trading Chart")
    asset_timeframe = st.selectbox(
        "Demo timeframe",
        options=CHART_CONFIG["topbar"]["timeframes"],
        index=CHART_CONFIG["topbar"]["timeframes"].index("1D"),
        key="asset_demo_chart_timeframe",
    )
    try:
        asset_chart = TradingChart(str(_raw_symbol), asset_timeframe)
        asset_chart.add_indicator("sma", length=20).add_indicator("ema", length=50).add_indicator("rsi")
        asset_chart.add_indicator("macd").add_indicator("bollinger_bands", length=20, std_dev=2.0).add_indicator("vwap")
        components.html(asset_chart.render_html(), height=CHART_CONFIG["size"]["height"])
    except ValueError as exc:
        st.warning(str(exc))
    except ModuleNotFoundError as exc:
        st.info(str(exc))
    except Exception as exc:
        st.error(f"Chart unavailable: {exc}")
