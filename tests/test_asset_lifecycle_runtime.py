"""Unit tests for app.runtime.asset_lifecycle (FB-AP-005)."""

from __future__ import annotations

from app.contracts.asset_lifecycle import AssetLifecycleState
from app.contracts.asset_model_manifest import AssetModelManifest
from app.runtime import asset_lifecycle as life
from app.runtime import asset_model_registry as reg


def test_effective_state_uninitialized(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(life, "_DEFAULT_LIFECYCLE_DIR", tmp_path)
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path / "m")
    assert life.effective_state("ABC-USD") == AssetLifecycleState.uninitialized


def test_effective_state_from_manifest_only(tmp_path, monkeypatch) -> None:
    mdir = tmp_path / "manifests"
    mdir.mkdir()
    monkeypatch.setattr(reg, "_DEFAULT_DIR", mdir)
    monkeypatch.setattr(life, "_DEFAULT_LIFECYCLE_DIR", tmp_path / "lc")
    reg.save_manifest(AssetModelManifest(canonical_symbol="Z-USD", forecaster_weights_path="/a.npz"))
    assert life.effective_state("Z-USD") == AssetLifecycleState.initialized_not_active
