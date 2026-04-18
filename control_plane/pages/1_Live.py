import streamlit as st

from control_plane._theme import inject_global_css
from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import api_get_json, get_api_base, require_streamlit_app_access

require_streamlit_app_access()
inject_global_css()
render_app_sidebar()
st.header("Live")
try:
    st.json(api_get_json("/status"))
except Exception as e:
    st.warning(f"Could not reach control plane: {e}")
st.caption(f"API: `{get_api_base()}`")
