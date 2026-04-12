"""merge_settings_for_execution (FB-UX-007)."""

from __future__ import annotations

from pathlib import Path

import pytest
from pydantic import SecretStr

from app.config.settings import AppSettings
from app.runtime.execution_settings_merge import merge_settings_for_execution
from app.runtime import user_store as us
from app.runtime import user_venue_credentials as uvc


def test_merge_paper_uses_alpaca_from_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_MULTI_TENANT_EXECUTION_CREDENTIALS", "true")
    db = tmp_path / "u.sqlite"
    master = "x" * 40
    r = us.create_user(db, "a@b.co", "password-88")
    uvc.save_credentials(
        db,
        master,
        r.id,
        alpaca_api_key="pk1",
        alpaca_api_secret="sec1",
    )
    base = AppSettings(
        auth_users_db_path=db,
        auth_venue_credentials_master_secret=SecretStr(master),
        execution_mode="paper",
    )
    m = merge_settings_for_execution(base, r.id)
    assert m.alpaca_api_key
    assert m.alpaca_api_key.get_secret_value() == "pk1"
