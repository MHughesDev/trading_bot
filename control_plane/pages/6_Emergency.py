import streamlit as st

from control_plane._theme import inject_global_css
from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import api_post_json, get_control_plane_key, require_streamlit_app_access

require_streamlit_app_access()
inject_global_css()
render_app_sidebar()
st.header("Emergency")
if st.button("Request FLATTEN (calls POST /flatten)"):
    try:
        st.json(api_post_json("/flatten", {}))
    except Exception as e:
        st.error(str(e))
st.caption("Set `NM_CONTROL_PLANE_API_KEY` if the API requires it.")
if not get_control_plane_key():
    st.info("No `NM_CONTROL_PLANE_API_KEY` set — flatten may fail if the API enforces auth.")
