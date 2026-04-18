import streamlit as st

from control_plane._theme import inject_global_css
from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import api_get_json, require_streamlit_app_access

require_streamlit_app_access()
inject_global_css()
render_app_sidebar()
st.header("Routes")
try:
    st.json(api_get_json("/routes"))
except Exception as e:
    st.warning(str(e))
