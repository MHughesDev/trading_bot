"""Strategy Builder — create your own trading strategies, no code required (FB-AP-XXX).

Deep link: ``http://localhost:8501/StrategyBuilder`` (path is ``/StrategyBuilder`` from
``pages/StrategyBuilder.py``).
"""

from __future__ import annotations

import streamlit as st

from control_plane._theme import inject_global_css
from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import require_streamlit_app_access
from control_plane.strategy_builder_page import render_strategy_builder_page

require_streamlit_app_access()
inject_global_css()
render_app_sidebar()
st.title("Strategy Builder")
render_strategy_builder_page()
