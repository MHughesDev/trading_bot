"""Asset Page manifest / model card (FB-UX-010) — read-only summary from ``GET /assets/models/{symbol}``."""

from __future__ import annotations

from typing import Any


def _cell(v: Any) -> str:
    if v is None:
        return "—"
    s = str(v).strip()
    return s if s else "—"


# Display order: training timestamps, then artifact paths, then instance id.
_MANIFEST_CARD_FIELDS: tuple[tuple[str, str], ...] = (
    ("Last trained (forecaster)", "forecaster_last_trained_at"),
    ("Last trained (RL)", "rl_last_trained_at"),
    ("Forecaster (torch)", "forecaster_torch_path"),
    ("Forecaster (weights)", "forecaster_weights_path"),
    ("Forecaster (conformal)", "forecaster_conformal_state_path"),
    ("Forecaster (config)", "forecaster_config_path"),
    ("Policy (MLP)", "policy_mlp_path"),
    ("Policy (checkpoint)", "policy_checkpoint_path"),
    ("Runtime instance id", "runtime_instance_id"),
)


def manifest_model_rows(manifest: dict[str, Any]) -> list[dict[str, str]]:
    """
    Rows for a read-only **Model / manifest** table from API JSON (``AssetModelManifest`` fields).
    """
    rows: list[dict[str, str]] = []
    for label, key in _MANIFEST_CARD_FIELDS:
        rows.append({"Field": label, "Value": _cell(manifest.get(key))})
    return rows
