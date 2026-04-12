import streamlit as st

from control_plane.streamlit_util import api_get_json, require_streamlit_app_access

require_streamlit_app_access()
st.header("Models")
try:
    st.json(api_get_json("/models"))
except Exception as e:
    st.warning(str(e))
