"""Asset Page — one canonical symbol (FB-AP-027).

Deep link example: ``http://localhost:8501/Asset?symbol=BTC-USD`` (path is ``/Asset`` from
``pages/Asset.py``).
"""

from __future__ import annotations

import streamlit as st

from control_plane._theme import inject_global_css
from control_plane.asset_page import render_asset_page_or_pick
from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import require_streamlit_app_access

require_streamlit_app_access()
inject_global_css()
render_app_sidebar()
st.title("Asset")
render_asset_page_or_pick()
