"""Shared Streamlit page registry."""

from __future__ import annotations

import streamlit as st

DASHBOARD_PAGE = st.Page(
    "pages/Dashboard.py",
    title="Dashboard",
    icon=":material/dashboard:",
    default=True,
)
ASSET_PAGE = st.Page("pages/Asset.py", title="Asset page", icon=":material/show_chart:")
STRATEGY_BUILDER_PAGE = st.Page(
    "pages/StrategyBuilder.py", title="Strategy builder", icon=":material/build:"
)
ACCOUNT_PAGE = st.Page("pages/7_Account.py", title="Account", icon=":material/settings:")
SIGN_IN_PAGE = st.Page("pages/0_Login.py", title="Sign in", icon=":material/login:")
SIGN_UP_PAGE = st.Page("pages/99_Sign_up.py", title="Sign up", icon=":material/person_add:")
LIVE_PAGE = st.Page("pages/1_Live.py", title="Live")
REGIMES_PAGE = st.Page("pages/2_Regimes.py", title="Regimes")
ROUTES_PAGE = st.Page("pages/3_Routes.py", title="Routes")
MODELS_PAGE = st.Page("pages/4_Models.py", title="Models")
LOGS_PAGE = st.Page("pages/5_Logs.py", title="Logs")
EMERGENCY_PAGE = st.Page("pages/6_Emergency.py", title="Emergency")
SETUP_API_KEYS_PAGE = st.Page("pages/98_Setup_API_keys.py", title="Setup API keys")

NAVIGATION_PAGES = [
    DASHBOARD_PAGE,
    ASSET_PAGE,
    STRATEGY_BUILDER_PAGE,
    ACCOUNT_PAGE,
    SIGN_IN_PAGE,
    SIGN_UP_PAGE,
    LIVE_PAGE,
    REGIMES_PAGE,
    ROUTES_PAGE,
    MODELS_PAGE,
    LOGS_PAGE,
    EMERGENCY_PAGE,
    SETUP_API_KEYS_PAGE,
]
