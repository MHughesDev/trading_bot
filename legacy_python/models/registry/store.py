"""
JSON registry for operator-facing "active model set" (FB-SPEC-06).

This does **not** replace `AppSettings` / `NM_*` — runtime still loads env/YAML.
The registry file documents intent and can be merged into `GET /status` → `model_artifacts`
for a single place to compare **declared** vs **env-configured** paths.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

REGISTRY_SCHEMA_VERSION = 1


def default_registry_path() -> Path:
    return Path("models") / "registry" / "active_model_set.json"


def read_active_model_set(path: str | Path | None) -> dict[str, Any] | None:
    """Return parsed registry or None if path missing/unreadable."""
    if path is None:
        return None
    p = Path(path)
    if not p.is_file():
        return None
    try:
        raw = json.loads(p.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(raw, dict):
        return None
    return raw


def write_active_model_set(path: str | Path, body: dict[str, Any]) -> None:
    """Write registry with schema version (operators / tooling)."""
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    out = dict(body)
    out.setdefault("schema_version", REGISTRY_SCHEMA_VERSION)
    p.write_text(json.dumps(out, indent=2), encoding="utf-8")


def merge_registry_into_serving_view(
    *,
    registry: dict[str, Any] | None,
    env_forecaster_weights: str | None,
    env_policy_mlp: str | None,
    env_conformal: str | None,
    env_lineage_id: str | None,
) -> dict[str, Any]:
    """
    Build a comparison view: registry-declared paths vs current env (truncated paths ok).

    Drift = any path field differs when both sides are non-empty strings.
    """
    if not registry:
        return {"registry_loaded": False}

    def _g(key: str) -> str | None:
        v = registry.get(key)
        if v is None or v == "":
            return None
        return str(v)

    reg_fw = _g("forecaster_weights_npz_path")
    reg_pp = _g("policy_mlp_npz_path")
    reg_cf = _g("forecaster_conformal_state_path")
    reg_id = _g("forecaster_checkpoint_id")

    drift: list[str] = []
    pairs = [
        ("forecaster_weights_npz_path", reg_fw, env_forecaster_weights),
        ("policy_mlp_npz_path", reg_pp, env_policy_mlp),
        ("forecaster_conformal_state_path", reg_cf, env_conformal),
        ("forecaster_checkpoint_id", reg_id, env_lineage_id),
    ]
    for name, a, b in pairs:
        if a and b and a != b:
            drift.append(f"{name}: registry={a!r} env={b!r}")

    return {
        "registry_loaded": True,
        "schema_version": registry.get("schema_version"),
        "updated_at": registry.get("updated_at"),
        "notes": registry.get("notes"),
        "declared": {
            "forecaster_checkpoint_id": reg_id,
            "forecaster_weights_npz_path": reg_fw,
            "policy_mlp_npz_path": reg_pp,
            "forecaster_conformal_state_path": reg_cf,
        },
        "drift_vs_env": drift,
        "aligned_with_env": len(drift) == 0,
    }
