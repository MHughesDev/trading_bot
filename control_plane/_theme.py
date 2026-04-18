"""Shared Streamlit visual system helpers (FB-UX-019)."""

from __future__ import annotations

THEME_CSS = """
<style>
@import url('https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&family=JetBrains+Mono:wght@400;500;600&display=swap');

:root {
  --pnl-up: #22D3A0;
  --pnl-down: #F87171;
}

html, body, [class*="css"] {
  font-family: 'Inter', sans-serif;
}

.stMarkdown code,
.stMetric-value,
[data-testid="stMetricValue"],
[data-testid="stMetricDelta"],
[data-testid="stCaptionContainer"] code {
  font-family: 'JetBrains Mono', monospace !important;
  font-variant-numeric: tabular-nums;
}

#MainMenu,
footer,
header,
[data-testid="stToolbar"],
[data-testid="stDecoration"],
[data-testid="stStatusWidget"],
button[kind="header"] {
  visibility: hidden;
  height: 0;
  position: fixed;
}

.tb-card {
  background: #111827;
  border: 1px solid #1F2937;
  border-radius: 12px;
  padding: 24px;
}

.tb-sidebar-brand {
  font-family: 'JetBrains Mono', monospace;
  font-size: 13px;
  letter-spacing: 0.02em;
  color: #F3F4F6;
  margin: 4px 0 2px 0;
}

.tb-sidebar-brand-mark {
  color: #9CA3AF;
  margin-right: 6px;
}

.tb-auth-wrap {
  max-width: 420px;
  margin: 6vh auto 0 auto;
}

.tb-auth-card {
  background: #111827;
  border: 1px solid #1F2937;
  border-radius: 12px;
  padding: 24px;
}

.tb-brand-lockup {
  text-align: center;
  font-family: 'JetBrains Mono', monospace;
  font-size: 16px;
  color: #F3F4F6;
}

.tb-brand-subtitle {
  text-align: center;
  color: #9CA3AF;
  font-size: 13px;
}

.tb-section-eyebrow {
  color: #6B7280;
  font-size: 11px;
  letter-spacing: .08em;
  font-weight: 600;
}

.tb-card {
  margin-bottom: 16px;
}
</style>
""".strip()


def inject_global_css() -> str:
    """Inject the app-wide CSS contract and return the CSS payload for tests."""
    import streamlit as st

    st.markdown(THEME_CSS, unsafe_allow_html=True)
    return THEME_CSS


def render_brand() -> None:
    """Small sidebar brand lockup for the navigation rail."""
    import streamlit as st

    st.sidebar.markdown(
        "<div class='tb-sidebar-brand'><span class='tb-sidebar-brand-mark'>◧</span>trading_bot</div>",
        unsafe_allow_html=True,
    )


def render_brand_lockup(*, subtitle: str | None = None) -> None:
    """Centered brand lockup for login/sign-up cards."""
    import streamlit as st

    st.markdown("<div class='tb-brand-lockup'>◧ trading_bot</div>", unsafe_allow_html=True)
    if subtitle:
        st.markdown(f"<div class='tb-brand-subtitle'>{subtitle}</div>", unsafe_allow_html=True)
