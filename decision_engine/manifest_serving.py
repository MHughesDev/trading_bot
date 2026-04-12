"""Per-asset manifest → effective `AppSettings` for model serving (FB-AP-003 / FB-AP-004)."""

from __future__ import annotations

from pathlib import Path

from app.config.settings import AppSettings
from app.contracts.asset_model_manifest import AssetModelManifest
from app.runtime.asset_model_registry import load_manifest


def resolve_manifest_serving_settings(
    base: AppSettings, symbol: str
) -> tuple[AppSettings, AssetModelManifest | None]:
    """
    When a registry manifest exists for ``symbol``, build serving settings from **manifest fields only**
    for forecaster/policy paths (no fallback to global ``NM_*`` for those keys), so one process cannot
    silently apply another symbol's global checkpoint to this symbol.
    """
    sym = symbol.strip()
    manifest = load_manifest(sym)
    if manifest is None:
        return base, None

    updates: dict[str, str | None] = {
        "models_forecaster_weights_path": manifest.forecaster_weights_path or None,
        "models_forecaster_conformal_state_path": manifest.forecaster_conformal_state_path or None,
        "models_forecaster_torch_path": manifest.forecaster_torch_path or None,
        "models_policy_mlp_path": manifest.policy_mlp_path or None,
    }
    return base.model_copy(update=updates), manifest


def forecaster_artifacts_resolved(settings: AppSettings) -> bool:
    """True if a concrete forecaster artifact file is available (torch or NPZ)."""
    ft = settings.models_forecaster_torch_path
    if ft and Path(ft).is_file():
        return True
    fw = settings.models_forecaster_weights_path
    return bool(fw and Path(fw).is_file())


def policy_manifest_path_broken(settings: AppSettings, manifest: AssetModelManifest) -> bool:
    """Manifest requests a policy NPZ path that is missing on disk."""
    p = (manifest.policy_mlp_path or "").strip()
    if not p:
        return False
    mp = settings.models_policy_mlp_path
    return not (bool(mp) and Path(mp).is_file())
