"""Shared Streamlit helpers: API base URL and simple HTTP helpers."""

from __future__ import annotations

import os
from typing import Any

import httpx

from control_plane.auth_cookie import session_cookie_name, session_token_from_httpx_response


def _operator_session_token() -> str | None:
    """Opaque session token stored after Streamlit login (FB-UX-003)."""
    try:
        import streamlit as st
    except ImportError:
        return None
    v = st.session_state.get("operator_session_token")
    return str(v).strip() if v else None


def _cookie_headers() -> dict[str, str]:
    tok = _operator_session_token()
    if not tok:
        return {}
    return {"Cookie": f"{session_cookie_name()}={tok}"}


def get_api_base() -> str:
    return os.getenv("NM_CONTROL_PLANE_URL", "http://127.0.0.1:8000").rstrip("/")


def get_control_plane_key() -> str:
    return os.getenv("NM_CONTROL_PLANE_API_KEY", "")


def get_grafana_url() -> str:
    return os.getenv("NM_GRAFANA_URL", "http://127.0.0.1:3000").rstrip("/")


def get_loki_url() -> str:
    return os.getenv("NM_LOKI_URL", "http://127.0.0.1:3100").rstrip("/")


def get_questdb_console_url() -> str:
    return os.getenv("NM_QUESTDB_CONSOLE_URL", "http://127.0.0.1:9000").rstrip("/")


def streamlit_route_guard_enabled() -> bool:
    """FB-UX-004: when true, app pages require session or ``NM_CONTROL_PLANE_API_KEY`` in the Streamlit process."""
    return os.getenv("NM_STREAMLIT_ROUTE_GUARD_ENABLED", "").strip().lower() in (
        "1",
        "true",
        "yes",
    )


def require_streamlit_app_access() -> None:
    """
    Redirect to the Login page unless guard is off, API key bypass is set, or ``GET /auth/me`` succeeds.

    Call **after** ``st.set_page_config`` on pages that use it. Login page must not call this.
    """
    import streamlit as st

    if not streamlit_route_guard_enabled():
        return
    if get_control_plane_key().strip():
        return
    if not _operator_session_token():
        st.switch_page("pages/0_Login.py")
        st.stop()
    try:
        r = httpx.get(
            f"{get_api_base()}/auth/me",
            timeout=12.0,
            headers=_cookie_headers() or None,
        )
    except httpx.HTTPError:
        st.session_state.pop("operator_session_token", None)
        st.switch_page("pages/0_Login.py")
        st.stop()
    if r.status_code != 200:
        st.session_state.pop("operator_session_token", None)
        st.switch_page("pages/0_Login.py")
        st.stop()


def api_get_json(path: str, *, timeout: float = 10.0) -> dict[str, Any]:
    h = _cookie_headers()
    r = httpx.get(f"{get_api_base()}{path}", timeout=timeout, headers=h or None)
    r.raise_for_status()
    return r.json()


def _mutate_headers() -> dict[str, str]:
    headers: dict[str, str] = {}
    key = get_control_plane_key()
    if key:
        headers["X-API-Key"] = key
    headers.update(_cookie_headers())
    return headers


def api_post_json(
    path: str,
    body: dict[str, Any] | None = None,
    *,
    timeout: float = 15.0,
    require_key: bool = True,
) -> dict[str, Any]:
    headers = _mutate_headers() if require_key else {}
    r = httpx.post(
        f"{get_api_base()}{path}",
        json=body or {},
        timeout=timeout,
        headers=headers,
    )
    r.raise_for_status()
    return r.json()


def api_put_json(
    path: str,
    body: dict[str, Any] | None = None,
    *,
    timeout: float = 15.0,
    require_key: bool = True,
) -> dict[str, Any]:
    headers = _mutate_headers() if require_key else {}
    r = httpx.put(
        f"{get_api_base()}{path}",
        json=body or {},
        timeout=timeout,
        headers=headers,
    )
    r.raise_for_status()
    return r.json()


def api_delete_json(path: str, *, timeout: float = 15.0, require_key: bool = True) -> dict[str, Any]:
    headers = _mutate_headers() if require_key else {}
    r = httpx.delete(f"{get_api_base()}{path}", timeout=timeout, headers=headers)
    r.raise_for_status()
    return r.json()


def operator_login(email: str, password: str) -> tuple[bool, str]:
    """POST /auth/login and store opaque session token in Streamlit session state (FB-UX-003)."""
    import streamlit as st

    r = httpx.post(
        f"{get_api_base()}/auth/login",
        json={"email": email, "password": password},
        timeout=20.0,
    )
    if r.status_code == 401:
        return False, "Invalid email or password"
    if r.status_code == 403:
        return (
            False,
            "Session auth is disabled on the API (set NM_AUTH_SESSION_ENABLED=true).",
        )
    if r.status_code >= 400:
        detail = r.json().get("detail", r.text) if r.headers.get("content-type", "").startswith(
            "application/json"
        ) else r.text
        return False, str(detail)
    tok = session_token_from_httpx_response(r)
    if not tok:
        return False, "Login succeeded but no session cookie was returned"
    st.session_state["operator_session_token"] = tok
    return True, ""


def operator_logout() -> None:
    """POST /auth/logout and clear stored session token."""
    import streamlit as st

    h = _cookie_headers()
    try:
        httpx.post(
            f"{get_api_base()}/auth/logout",
            timeout=15.0,
            headers=h or None,
        )
    except httpx.HTTPError:
        pass
    st.session_state.pop("operator_session_token", None)


def operator_register(email: str, password: str) -> tuple[bool, str]:
    """POST /auth/register (no auth required)."""
    r = httpx.post(
        f"{get_api_base()}/auth/register",
        json={"email": email, "password": password},
        timeout=20.0,
    )
    if r.status_code == 409:
        return False, "That email is already registered"
    if r.status_code == 422:
        try:
            d = r.json().get("detail")
        except Exception:
            d = r.text
        return False, str(d)
    if r.status_code >= 400:
        return False, r.text
    return True, ""
