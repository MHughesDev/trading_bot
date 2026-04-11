"""
Unified model artifact contract (FB-SPEC-03).

Single JSON-serializable view of **serving** paths (hot path) vs **training-only** artifacts
so operators are not confused by `forecaster_quantile_real.joblib` vs NumPy `ForecasterModel`.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from app.config.settings import AppSettings

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
    return {
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
