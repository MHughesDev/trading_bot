"""
Register per-asset manifest after successful init (FB-AP-012).

Writes :class:`app.contracts.asset_model_manifest.AssetModelManifest` with paths under the init
run directory (forecaster torch + policy NPZ when present). **Lifecycle state** ``initialized_not_active``
is tracked separately (FB-AP-005); this slice only persists the artifact manifest.
"""

from __future__ import annotations

from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from app.contracts.asset_model_manifest import AssetModelManifest
from app.runtime.asset_model_registry import save_manifest


def register_init_artifacts_manifest(
    *,
    symbol: str,
    job_id: str,
    run_dir: Path,
) -> dict[str, Any]:
    """
    Create or replace manifest for ``symbol`` pointing at ``run_dir/forecaster`` and ``run_dir/policy`` files.
    """
    sym = symbol.strip()
    ft = run_dir / "forecaster" / "forecaster_torch.pt"
    pm = run_dir / "policy" / "policy_mlp.npz"
    now = datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")

    m = AssetModelManifest(
        canonical_symbol=sym,
        forecaster_torch_path=str(ft.resolve()) if ft.is_file() else None,
        policy_mlp_path=str(pm.resolve()) if pm.is_file() else None,
        forecaster_last_trained_at=now if ft.is_file() else None,
        rl_last_trained_at=now if pm.is_file() else None,
        runtime_instance_id=f"init:{job_id}",
    )
    path = save_manifest(m)
    return {
        "symbol": sym,
        "manifest_path": str(path.resolve()),
        "forecaster_torch_path": m.forecaster_torch_path,
        "policy_mlp_path": m.policy_mlp_path,
        "job_id": job_id,
    }


def init_register_detail_payload(payload: dict[str, Any]) -> dict[str, Any]:
    return {
        "symbol": payload.get("symbol"),
        "manifest_path": payload.get("manifest_path"),
        "forecaster_torch_path": payload.get("forecaster_torch_path"),
        "policy_mlp_path": payload.get("policy_mlp_path"),
    }
