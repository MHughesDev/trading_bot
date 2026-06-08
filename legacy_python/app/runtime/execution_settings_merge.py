"""Merge per-user venue credentials into :class:`AppSettings` for execution (FB-UX-007)."""

from __future__ import annotations

import os

from pydantic import SecretStr

from app.config.settings import AppSettings
from app.runtime import user_venue_credentials as uvc


def venue_master_secret(settings: AppSettings) -> str | None:
    s = settings.auth_venue_credentials_master_secret
    if not s:
        return None
    v = s.get_secret_value().strip()
    return v or None


def merge_settings_for_execution(base: AppSettings, user_id: int | None) -> AppSettings:
    """
    When multi-tenant execution credentials are enabled, overlay stored Alpaca/Coinbase secrets
    for the active session user (paper → Alpaca, live → Coinbase).
    """
    if user_id is None:
        return base
    if os.getenv("NM_MULTI_TENANT_EXECUTION_CREDENTIALS", "").strip().lower() not in (
        "1",
        "true",
        "yes",
    ):
        return base
    master = venue_master_secret(base)
    if not master:
        return base
    creds = uvc.load_decrypted_credentials(base.auth_users_db_path, master, user_id)
    updates: dict = {}
    if base.execution_mode == "paper":
        ak, asec = creds.get("alpaca_api_key"), creds.get("alpaca_api_secret")
        if ak and asec:
            updates["alpaca_api_key"] = SecretStr(ak)
            updates["alpaca_api_secret"] = SecretStr(asec)
    elif base.execution_mode == "live":
        ck, csec = creds.get("coinbase_api_key"), creds.get("coinbase_api_secret")
        if ck and csec:
            updates["coinbase_api_key"] = SecretStr(ck)
            updates["coinbase_api_secret"] = SecretStr(csec)
    if not updates:
        return base
    return base.model_copy(update=updates)
