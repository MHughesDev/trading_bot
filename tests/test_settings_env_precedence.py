from __future__ import annotations

from app.config.settings import load_settings


def test_env_overrides_yaml_for_auth_session_enabled(monkeypatch) -> None:
    monkeypatch.setenv("NM_AUTH_SESSION_ENABLED", "true")
    settings = load_settings()
    assert settings.auth_session_enabled is True
