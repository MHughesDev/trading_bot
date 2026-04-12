"""auth_cookie helpers (FB-UX-003)."""

from __future__ import annotations

import httpx

from control_plane.auth_cookie import session_token_from_httpx_response


def test_session_token_from_response(monkeypatch):
    monkeypatch.setenv("NM_AUTH_SESSION_COOKIE_NAME", "tb_operator_session")
    r = httpx.Response(200, request=httpx.Request("POST", "http://x/"))
    r.cookies.set("tb_operator_session", "abc", domain="127.0.0.1")
    assert session_token_from_httpx_response(r) == "abc"
