import streamlit as st

from control_plane.streamlit_util import api_get_json

st.header("Routes")
try:
    st.json(api_get_json("/routes"))
except Exception as e:
    st.warning(str(e))
