"""Per-user data path layout (FB-UX-007)."""

from __future__ import annotations

import pytest

from app.runtime import tenant_context as tc
from app.runtime import user_data_paths as udp


def test_under_data_no_tenant(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_MULTI_TENANT_DATA_SCOPING", "true")
    monkeypatch.delenv("NM_ASSET_MODEL_REGISTRY_DIR", raising=False)
    tc.set_current_user_id(None)
    p = udp.registry_manifests_dir()
    assert "asset_model_registry" in str(p)
    assert "users" not in str(p)


def test_under_data_with_tenant(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_MULTI_TENANT_DATA_SCOPING", "true")
    monkeypatch.delenv("NM_ASSET_MODEL_REGISTRY_DIR", raising=False)
    tc.set_current_user_id(7)
    try:
        p = udp.registry_manifests_dir()
    finally:
        tc.set_current_user_id(None)
    assert "users" in str(p)
    assert "7" in str(p)
