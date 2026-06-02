"""Phase E / P2: active-set manifest binds forecaster_quantile_path and forecaster_torch_path."""

from __future__ import annotations

import json
from pathlib import Path

from models.registry.active_set import _MANIFEST_MODEL_KEYS, apply_active_model_set
from app.config.settings import load_settings


def test_manifest_keys_include_quantile_and_torch() -> None:
    assert "forecaster_quantile_path" in _MANIFEST_MODEL_KEYS
    assert "forecaster_torch_path" in _MANIFEST_MODEL_KEYS


def test_active_set_applies_quantile_path(tmp_path) -> None:
    manifest = {
        "forecaster_quantile_path": str(tmp_path / "q.joblib"),
        "forecaster_torch_path": str(tmp_path / "torch.pt"),
    }
    manifest_path = tmp_path / "active_set.json"
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

    settings = load_settings()
    settings = settings.model_copy(update={"models_active_set_path": str(manifest_path)})
    result = apply_active_model_set(settings)

    assert result.models_forecaster_quantile_path == str(tmp_path / "q.joblib")
    assert result.models_forecaster_torch_path == str(tmp_path / "torch.pt")


def test_asset_model_manifest_has_forecaster_quantile_path() -> None:
    from app.contracts.asset_model_manifest import AssetModelManifest

    m = AssetModelManifest(
        canonical_symbol="BTC-USD",
        forecaster_quantile_path="/tmp/q.joblib",
    )
    assert m.forecaster_quantile_path == "/tmp/q.joblib"
