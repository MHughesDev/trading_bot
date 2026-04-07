"""Streamlit dashboard entry — optional dependency `pip install streamlit`."""

from __future__ import annotations


def main() -> None:
    try:
        import streamlit as st
    except ImportError:
        raise SystemExit("Install streamlit: pip install streamlit") from None

    st.set_page_config(page_title="NautilusMonster", layout="wide")
    st.title("NautilusMonster V3")
    st.write("Live · Regimes · Routes · Models · Logs · Emergency — wire to control plane API.")


if __name__ == "__main__":
    main()
