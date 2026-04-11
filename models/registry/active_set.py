"""Filesystem-backed active model set (FB-SPEC-06).

Optional JSON manifest pointed to by ``NM_MODELS_ACTIVE_SET_PATH`` / ``models.active_set_path``.
When present and valid, its non-null fields override serving paths on ``AppSettings`` after YAML + env
so operators have a single promoted artifact set without ad-hoc env churn.
"""

from __future__ import annotations

import json
import logging
from pathlib import Path
from typing import Any

from app.config.settings import AppSettings

logger = logging.getLogger(__name__)

# Keys aligned with ``default.yaml`` ``models:`` section (values are merged onto AppSettings).
_MANIFEST_MODEL_KEYS = (
    "forecaster_checkpoint_id",
    "forecaster_conformal_state_path",
    "forecaster_weights_path",
    "policy_mlp_path",
)


def _read_manifest(path: Path) -> dict[str, Any] | None:
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except OSError as e:
        logger.error("active model set: cannot read %s: %s", path, e)
        return None
    except json.JSONDecodeError as e:
        logger.error("active model set: invalid JSON in %s: %s", path, e)
        return None
    if not isinstance(raw, dict):
        logger.error("active model set: root must be a JSON object: %s", path)
        return None
    return raw


def apply_active_model_set(settings: AppSettings) -> AppSettings:
    """
    If ``models_active_set_path`` points to a readable JSON file, merge known keys onto ``settings``.

    Manifest overrides YAML/env for those keys (promotion / single source of truth for serving paths).
    """
    ap = settings.models_active_set_path
    if not ap:
        return settings
    path = Path(ap)
    if not path.is_file():
        return settings

    data = _read_manifest(path)
    if data is None:
        return settings

    updates: dict[str, Any] = {}
    for k in _MANIFEST_MODEL_KEYS:
        if k not in data:
            continue
        val = data[k]
        attr = f"models_{k}"
        if val is None:
            updates[attr] = None
        else:
            updates[attr] = str(val).strip() or None

    if "label" in data and data["label"] is not None:
        updates["models_active_set_label"] = str(data["label"]).strip() or None

    ver = data.get("version")
    if ver is not None:
        try:
            updates["models_active_set_manifest_version"] = int(ver)
        except (TypeError, ValueError):
            logger.warning("active model set: ignored non-integer version in %s", path)

    if not updates:
        return settings

    return settings.model_copy(update=updates)


def active_model_set_status(settings: AppSettings) -> dict[str, Any]:
    """Operator-facing snapshot for ``GET /status`` / ``model_artifacts``."""
    path_str = settings.models_active_set_path
    path = Path(path_str) if path_str else None
    return {
        "manifest_path": path_str,
        "manifest_file_exists": bool(path and path.is_file()),
        "label": settings.models_active_set_label,
        "manifest_version": settings.models_active_set_manifest_version,
        "note": (
            "Serving paths on this process are merged: default.yaml + NM_* env + optional JSON manifest "
            "(manifest overrides env for keys it sets). Set NM_MODELS_ACTIVE_SET_PATH to a JSON file."
        ),
    }
