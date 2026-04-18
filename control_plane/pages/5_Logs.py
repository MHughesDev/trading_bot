import streamlit as st

from control_plane._theme import inject_global_css
from control_plane.streamlit_chrome import render_app_sidebar
from control_plane.streamlit_util import get_grafana_url, get_loki_url, require_streamlit_app_access

require_streamlit_app_access()
inject_global_css()
render_app_sidebar()
st.header("Logs")
st.markdown(
    f"- **Loki API:** [{get_loki_url()}]({get_loki_url()})\n"
    f"- **Grafana:** [{get_grafana_url()}]({get_grafana_url()})\n\n"
    "Promtail ships container logs to Loki when using `infra/docker-compose.yml`."
)
