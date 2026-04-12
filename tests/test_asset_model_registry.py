"""Per-asset model manifest registry (FB-AP-001 / FB-AP-002)."""

from __future__ import annotations

import pytest

from app.contracts.asset_model_manifest import AssetModelManifest
from app.runtime import asset_model_registry as reg


def test_manifest_validation_strips_symbol() -> None:
    m = AssetModelManifest(canonical_symbol="  BTC-USD  ")
    assert m.canonical_symbol == "BTC-USD"


def test_validate_manifest_symbol_mismatch() -> None:
    m = AssetModelManifest(canonical_symbol="ETH-USD")
    with pytest.raises(ValueError, match="does not match"):
        reg.validate_manifest_symbol("BTC-USD", m)


def test_save_load_roundtrip(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path)
    m = AssetModelManifest(
        canonical_symbol="BTC-USD",
        forecaster_torch_path="/tmp/forecaster_torch.pt",
        policy_mlp_path="/tmp/policy.npz",
    )
    reg.save_manifest(m)
    p = tmp_path / "BTC-USD.json"
    assert p.is_file()
    loaded = reg.load_manifest("BTC-USD")
    assert loaded is not None
    assert loaded.canonical_symbol == "BTC-USD"
    assert loaded.forecaster_torch_path == "/tmp/forecaster_torch.pt"


def test_list_symbols_sorted(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path)
    reg.save_manifest(AssetModelManifest(canonical_symbol="AAA-USD"))
    reg.save_manifest(AssetModelManifest(canonical_symbol="ZZZ-USD"))
    assert reg.list_symbols() == ["AAA-USD", "ZZZ-USD"]


def test_delete_manifest(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path)
    reg.save_manifest(AssetModelManifest(canonical_symbol="BTC-USD"))
    assert reg.delete_manifest("BTC-USD") is True
    assert reg.load_manifest("BTC-USD") is None
    assert reg.delete_manifest("BTC-USD") is False


def test_invalid_path_symbol_rejected(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path)
    with pytest.raises(ValueError):
        reg._manifest_path("x/y")
