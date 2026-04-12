import streamlit as st

from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import get_questdb_console_url, require_streamlit_app_access

require_streamlit_app_access()
render_app_sidebar()
st.header("Regimes")
st.markdown(
    f"Open the **QuestDB console** to query `decision_traces` and `bars`: "
    f"[{get_questdb_console_url()}]({get_questdb_console_url()})"
)
st.caption("HMM regime labels appear in decision traces when persisted.")
