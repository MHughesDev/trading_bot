"""Asset Page — one canonical symbol (FB-AP-027).

Deep link example: ``http://localhost:8501/Asset?symbol=BTC-USD`` (path is ``/Asset`` from
``pages/Asset.py``).
"""

from __future__ import annotations

import streamlit as st

from control_plane.asset_page import render_asset_page_or_pick

st.title("Asset")
render_asset_page_or_pick()
