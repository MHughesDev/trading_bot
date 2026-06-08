"""FB-UX-022 auth/account layout smoke checks."""

from __future__ import annotations

from pathlib import Path


def _read(rel: str) -> str:
    return Path(rel).read_text(encoding="utf-8")


def test_login_uses_collapsed_sidebar_and_auth_card() -> None:
    src = _read("control_plane/pages/0_Login.py")
    assert "initial_sidebar_state=\"collapsed\"" in src
    assert "tb-auth-card" in src
    assert "render_brand_lockup" in src


def test_signup_uses_collapsed_sidebar_and_auth_card() -> None:
    src = _read("control_plane/pages/99_Sign_up.py")
    assert "initial_sidebar_state=\"collapsed\"" in src
    assert "tb-auth-card" in src
    assert "render_brand_lockup" in src


def test_account_uses_card_wrappers_and_section_eyebrows() -> None:
    src = _read("control_plane/pages/7_Account.py")
    assert "tb-card" in src
    assert "tb-section-eyebrow" in src
    assert "VENUE KEYS" in src
