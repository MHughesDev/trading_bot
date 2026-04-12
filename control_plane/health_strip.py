"""Dashboard health / preflight strip (FB-UX-008) — surfaces ``GET /status`` subsets."""

from __future__ import annotations

from typing import Any

from control_plane.streamlit_util import api_get_json


def _fmt_issues(items: list[Any], *, max_items: int = 3) -> str:
    if not items:
        return ""
    lines = [str(x) for x in items[:max_items]]
    if len(items) > max_items:
        lines.append(f"… +{len(items) - max_items} more")
    return " · ".join(lines)


def render_dashboard_health_strip() -> None:
    """One glanceable row: preflight, production preflight, execution profile (from ``/status``)."""
    import streamlit as st

    try:
        st_data = api_get_json("/status")
    except Exception as e:
        st.error(f"Health strip: could not load **`GET /status`**: {e}")
        return

    pf = st_data.get("preflight") or {}
    pp = st_data.get("production_preflight") or {}
    ep = st_data.get("execution_profile") or {}

    sev = str(pf.get("severity") or "?")
    pf_ok = bool(pf.get("ok"))
    issues = pf.get("issues") or []
    warns = pf.get("warnings") or []

    prod_ok = bool(pp.get("ok"))
    signing = bool(pp.get("signing_secret_configured"))
    unsigned = bool(pp.get("unsigned_execution_allowed"))
    venue_ok = bool(pp.get("venue_credentials_configured"))

    legacy = ep.get("legacy_api_enabled")
    default_mode = ep.get("default_execution_mode") or ep.get("active_execution_mode") or "?"

    c1, c2, c3 = st.columns(3)
    with c1:
        st.markdown("**Preflight**")
        if pf_ok and sev == "ok":
            st.success("OK")
        elif pf_ok and sev == "warn":
            st.warning(f"Warn ({sev})")
        else:
            st.error(f"Block / issues ({sev})")
        detail = _fmt_issues(issues) or _fmt_issues(warns)
        if detail:
            st.caption(detail[:500])
    with c2:
        st.markdown("**Production preflight**")
        if prod_ok:
            st.success("OK")
        else:
            st.error("Not OK")
        st.caption(
            f"Signing: {'yes' if signing else 'no'} · "
            f"Unsigned allowed: {'yes' if unsigned else 'no'} · "
            f"Venue creds: {'yes' if venue_ok else 'no'}"
        )
    with c3:
        st.markdown("**Execution profile**")
        if legacy is False:
            st.info(f"Default mode (env): `{default_mode}` — per-asset overrides on Asset page.")
        elif legacy is True:
            st.success(f"Legacy API on — active: `{ep.get('active_execution_mode', '?')}`")
        else:
            st.caption(f"Default: `{default_mode}`")

    st.divider()
