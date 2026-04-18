"""FB-CAN-061: canonical metadata completeness and production validation."""

from __future__ import annotations

import pytest

from app.config.canonical_config import resolve_canonical_config
from app.config.settings import AppSettings


def _minimal_apex(env_scope: str = "research") -> dict:
    return {
        "apex_canonical": {
            "metadata": {"environment_scope": env_scope},
        }
    }


def test_live_rejects_unspecified_environment_scope(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_EXECUTION_MODE", "live")
    s = AppSettings()
    assert s.execution_mode == "live"
    raw = _minimal_apex("unspecified")
    with pytest.raises(ValueError, match="environment_scope"):
        resolve_canonical_config(s, raw)


def test_strict_env_rejects_unspecified(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_CANONICAL_CONFIG_STRICT", "1")
    s = AppSettings()
    raw = _minimal_apex("unspecified")
    with pytest.raises(ValueError, match="environment_scope"):
        resolve_canonical_config(s, raw)


def test_strict_allows_explicit_scope(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_CANONICAL_CONFIG_STRICT", "1")
    s = AppSettings()
    c = resolve_canonical_config(s, _minimal_apex("shadow"))
    assert c.metadata.environment_scope == "shadow"


def test_paper_allows_unspecified_without_strict(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("NM_CANONICAL_CONFIG_STRICT", raising=False)
    monkeypatch.delenv("NM_EXECUTION_MODE", raising=False)
    s = AppSettings()
    assert s.execution_mode == "paper"
    c = resolve_canonical_config(s, _minimal_apex("unspecified"))
    assert c.metadata.environment_scope == "unspecified"


def test_empty_enabled_feature_families_rejected() -> None:
    from pydantic import ValidationError

    from app.config.canonical_config import CanonicalMetadata

    with pytest.raises(ValidationError):
        CanonicalMetadata(
            config_version="1",
            config_name="x",
            created_at="2026-01-01T00:00:00+00:00",
            created_by="t",
            notes="n",
            enabled_feature_families=[],
        )
