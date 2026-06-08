"""
Unified model artifact contract (FB-SPEC-03).

Single JSON-serializable view of **serving** paths (hot path) vs **training-only** artifacts
so operators are not confused by `forecaster_quantile_real.joblib` vs NumPy `ForecasterModel`.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from app.config.settings import AppSettings
from models.registry.active_set import active_model_set_status
from models.registry.store import merge_registry_into_serving_view, read_active_model_set

# Campaign / nightly training default filename (sklearn QuantileRegressor) — not loaded by DecisionPipeline.
TRAINING_QUANTILE_FORECASTER_JOBLIB = "forecaster_quantile_real.joblib"


def model_artifact_contract(settings: AppSettings) -> dict[str, Any]:
    """
    Operator-facing snapshot: lineage labels, optional paths, file existence, and training disclaimer.

    No secrets; paths are echoed as configured (relative or absolute).
    """
    fw = settings.models_forecaster_weights_path
    pp = settings.models_policy_mlp_path
    conf = settings.models_forecaster_conformal_state_path
    reg_path = settings.models_active_registry_path
    registry = read_active_model_set(reg_path) if reg_path else read_active_model_set(
        Path("models") / "registry" / "active_model_set.json"
    )
    registry_view = merge_registry_into_serving_view(
        registry=registry,
        env_forecaster_weights=fw,
        env_policy_mlp=pp,
        env_conformal=conf,
        env_lineage_id=settings.models_forecaster_checkpoint_id,
    )

    return {
        "active_model_set": active_model_set_status(settings),
        "serving": {
            "lineage_checkpoint_id": settings.models_forecaster_checkpoint_id,
            "conformal_state_path": conf,
            "conformal_state_file_exists": bool(conf and Path(conf).is_file()),
            "forecaster_weights_npz_path": fw,
            "forecaster_weights_file_exists": bool(fw and Path(fw).is_file()),
            "policy_mlp_npz_path": pp,
            "policy_mlp_file_exists": bool(pp and Path(pp).is_file()),
            "forecaster_forward": "npz_weights"
            if (fw and Path(fw).is_file())
            else "numpy_rng",
            "policy_actor": "mlp_npz" if (pp and Path(pp).is_file()) else "heuristic",
        },
        "registry": {
            "config_path": reg_path,
            "active_model_set": registry_view,
        },
        "training": {
            "torch_device": settings.models_torch_device,
            "quantile_regressor_artifact": TRAINING_QUANTILE_FORECASTER_JOBLIB,
            "note": (
                f"Campaign writes `{TRAINING_QUANTILE_FORECASTER_JOBLIB}` (sklearn) under "
                "`NM_TRAINING_ARTIFACT_DIR`; runtime `ForecastPacket` uses NumPy `ForecasterModel` "
                "unless `NM_MODELS_FORECASTER_WEIGHTS_PATH` NPZ is set."
            ),
        },
    }


def load_training_report_score(path: Path) -> float | None:
    """Best-effort: read `best_forecaster_aggregate_score` from a training_report.json path."""
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(raw, dict):
        return None
    v = raw.get("best_forecaster_aggregate_score")
    if v is None:
        return None
    try:
        return float(v)
    except (TypeError, ValueError):
        return None


def resolve_champion_training_report_path(previous_champion_path: str) -> Path | None:
    """
    Map `NM_PREVIOUS_FORECASTER_CHAMPION_PATH` to a `training_report.json` for score comparison.

    Accepts: path to `training_report.json`, path to artifact dir, or path to `.joblib` (uses parent dir).
    """
    p = Path(previous_champion_path)
    if p.is_file():
        if p.name == "training_report.json":
            return p
        if p.suffix == ".joblib":
            candidate = p.parent / "training_report.json"
            return candidate if candidate.is_file() else None
        return None
    if p.is_dir():
        candidate = p / "training_report.json"
        return candidate if candidate.is_file() else None
    return None
