import streamlit as st

from control_plane.streamlit_util import get_questdb_console_url

st.header("Regimes")
st.markdown(
    f"Open the **QuestDB console** to query `decision_traces` and `bars`: "
    f"[{get_questdb_console_url()}]({get_questdb_console_url()})"
)
st.caption("HMM regime labels appear in decision traces when persisted.")
