import streamlit as st

from control_plane.streamlit_util import api_get_json, get_api_base, require_streamlit_app_access

require_streamlit_app_access()
st.header("Live")
try:
    st.json(api_get_json("/status"))
except Exception as e:
    st.warning(f"Could not reach control plane: {e}")
st.caption(f"API: `{get_api_base()}`")
