"""FB-AUTH-001: ensure page modules include route-gate calls."""

from __future__ import annotations

from pathlib import Path


PAGES_DIR = Path("control_plane/pages")


def _read(name: str) -> str:
    return (PAGES_DIR / name).read_text(encoding="utf-8")


def test_login_and_signup_do_not_require_full_app_gate() -> None:
    login = _read("0_Login.py")
    signup = _read("99_Sign_up.py")
    assert "require_streamlit_app_access()" not in login
    assert "require_streamlit_app_access()" not in signup
    assert "redirect_authenticated_user_from_auth_page()" in login
    assert "redirect_authenticated_user_from_auth_page()" in signup
    assert 'page_link("pages/99_Sign_up.py"' in login
    assert 'page_link("pages/0_Login.py"' in signup


def test_all_authenticated_pages_call_require_streamlit_app_access() -> None:
    exempt = {"0_Login.py", "99_Sign_up.py", "98_Setup_API_keys.py"}
    for page in PAGES_DIR.glob("*.py"):
        if page.name in exempt:
            continue
        src = page.read_text(encoding="utf-8")
        assert "require_streamlit_app_access()" in src, page.name


def test_setup_api_keys_uses_session_only_gate() -> None:
    setup = _read("98_Setup_API_keys.py")
    assert "require_streamlit_session_only()" in setup
