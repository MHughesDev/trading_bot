"""Helpers for operator session cookies (Streamlit ↔ FastAPI). FB-UX-003."""

from __future__ import annotations

import os

import httpx


def session_cookie_name() -> str:
    return os.getenv("NM_AUTH_SESSION_COOKIE_NAME", "tb_operator_session")


def session_token_from_httpx_response(response: httpx.Response, name: str | None = None) -> str | None:
    """Read session token from httpx response cookies (after POST /auth/login)."""
    cn = name or session_cookie_name()
    return response.cookies.get(cn)
