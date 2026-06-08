"""Per-asset lifecycle state (FB-AP-005)."""

from __future__ import annotations

import pytest

from app.contracts.asset_lifecycle import AssetLifecycleState
from app.contracts.asset_model_manifest import AssetModelManifest
from app.runtime import asset_lifecycle_state as lc
from app.runtime import asset_model_registry as reg


def _dirs(tmp_path):
    """Manifest and lifecycle stores must use different directories (both use `<sym>.json`)."""
    mdir = tmp_path / "manifests"
    ldir = tmp_path / "lifecycle"
    mdir.mkdir(parents=True, exist_ok=True)
    ldir.mkdir(parents=True, exist_ok=True)
    return mdir, ldir


def test_effective_uninitialized_without_manifest(tmp_path, monkeypatch) -> None:
    mdir, ldir = _dirs(tmp_path)
    monkeypatch.setattr(lc, "_DEFAULT_DIR", ldir)
    monkeypatch.setattr(reg, "_DEFAULT_DIR", mdir)
    assert lc.effective_lifecycle_state("BTC-USD") == AssetLifecycleState.uninitialized


def test_effective_initialized_not_active_with_manifest_only(tmp_path, monkeypatch) -> None:
    mdir, ldir = _dirs(tmp_path)
    monkeypatch.setattr(lc, "_DEFAULT_DIR", ldir)
    monkeypatch.setattr(reg, "_DEFAULT_DIR", mdir)
    reg.save_manifest(AssetModelManifest(canonical_symbol="BTC-USD"))
    assert lc.effective_lifecycle_state("BTC-USD") == AssetLifecycleState.initialized_not_active


def test_start_requires_manifest(tmp_path, monkeypatch) -> None:
    mdir, ldir = _dirs(tmp_path)
    monkeypatch.setattr(lc, "_DEFAULT_DIR", ldir)
    monkeypatch.setattr(reg, "_DEFAULT_DIR", mdir)
    with pytest.raises(ValueError, match="no model manifest"):
        lc.transition_start("BTC-USD")


def test_start_stop_roundtrip(tmp_path, monkeypatch) -> None:
    mdir, ldir = _dirs(tmp_path)
    monkeypatch.setattr(lc, "_DEFAULT_DIR", ldir)
    monkeypatch.setattr(reg, "_DEFAULT_DIR", mdir)
    reg.save_manifest(AssetModelManifest(canonical_symbol="BTC-USD"))
    assert lc.effective_lifecycle_state("BTC-USD") == AssetLifecycleState.initialized_not_active
    lc.transition_start("BTC-USD")
    assert lc.effective_lifecycle_state("BTC-USD") == AssetLifecycleState.active
    lc.transition_stop("BTC-USD")
    assert lc.effective_lifecycle_state("BTC-USD") == AssetLifecycleState.initialized_not_active


def test_orphan_state_file_removed_when_manifest_deleted(tmp_path, monkeypatch) -> None:
    mdir, ldir = _dirs(tmp_path)
    monkeypatch.setattr(lc, "_DEFAULT_DIR", ldir)
    monkeypatch.setattr(reg, "_DEFAULT_DIR", mdir)
    reg.save_manifest(AssetModelManifest(canonical_symbol="BTC-USD"))
    lc.set_active("BTC-USD")
    assert lc.effective_lifecycle_state("BTC-USD") == AssetLifecycleState.active
    reg.delete_manifest("BTC-USD")
    assert lc.effective_lifecycle_state("BTC-USD") == AssetLifecycleState.uninitialized
    assert not (lc.state_dir() / "BTC-USD.json").is_file()
