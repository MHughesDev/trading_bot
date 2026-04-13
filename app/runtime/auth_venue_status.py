"""Per-user venue (Alpaca / Coinbase) key completeness for auth responses and Streamlit gating."""

from __future__ import annotations

import os
from typing import Any

from app.config.settings import AppSettings
from app.runtime import user_venue_credentials as user_venue_credentials_mod


def streamlit_venue_keys_gate_enabled() -> bool:
    """When true (NM_STREAMLIT_VENUE_KEYS_REQUIRED), logged-in users must save venue keys before the app."""
    return os.getenv("NM_STREAMLIT_VENUE_KEYS_REQUIRED", "").strip().lower() in (
        "1",
        "true",
        "yes",
    )


def _master_configured(settings: AppSettings) -> bool:
    s = settings.auth_venue_credentials_master_secret
    if not s:
        return False
    return bool(s.get_secret_value().strip())


def user_has_required_venue_keys(settings: AppSettings, masked: dict[str, Any]) -> bool:
    """True if encrypted store has the key pair needed for the current default execution mode."""
    if (settings.execution_mode or "paper") == "paper":
        return bool(masked.get("alpaca_key_set") and masked.get("alpaca_secret_set"))
    return bool(masked.get("coinbase_key_set") and masked.get("coinbase_secret_set"))


def venue_keys_status_for_user(
    settings: AppSettings, user_id: int
) -> tuple[bool | None, bool | None]:
    """
    Returns (venue_keys_required, venue_keys_complete) for /auth/me and /auth/login.

    (None, None) when session auth is off — callers should omit or null these in JSON.
    When gating is off or master secret unset, returns (False, True) so clients do not block.
    """
    # Session-only responses use None to mean "not applicable"
    if not settings.auth_session_enabled:
        return None, None
    if not streamlit_venue_keys_gate_enabled() or not _master_configured(settings):
        return False, True
    master = settings.auth_venue_credentials_master_secret
    assert master is not None
    m = master.get_secret_value().strip()
    masked = user_venue_credentials_mod.load_masked(settings.auth_users_db_path, m, user_id)
    complete = user_has_required_venue_keys(settings, masked)
    return True, complete
