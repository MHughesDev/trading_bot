"""Canonical config metadata validation (FB-CAN-061, APEX spec §4).

Required metadata fields: ``config_version``, ``config_name``, ``created_at``, ``created_by``,
``environment_scope``, ``notes``, ``enabled_feature_families``; optional ``parent_config_version``.

Production / strict checks reject **unspecified** ``environment_scope`` when the operator opts in
via ``NM_CANONICAL_CONFIG_STRICT=1`` or when ``execution_mode`` is **live** (trading path).
"""

from __future__ import annotations

import os
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from app.config.canonical_config import CanonicalMetadata


def canonical_config_strict_enabled(execution_mode: str) -> bool:
    """True when merged canonical metadata must satisfy production rules."""
    if execution_mode.strip().lower() == "live":
        return True
    return os.environ.get("NM_CANONICAL_CONFIG_STRICT", "").strip().lower() in (
        "1",
        "true",
        "yes",
    )


def validate_canonical_metadata_complete(meta: "CanonicalMetadata") -> None:
    """Schema-level completeness (always enforced after :func:`resolve_canonical_config`)."""
    errs: list[str] = []
    if not (meta.config_version or "").strip():
        errs.append("metadata.config_version is empty")
    if not (meta.config_name or "").strip():
        errs.append("metadata.config_name is empty")
    if not (meta.created_at or "").strip():
        errs.append("metadata.created_at is empty")
    if not (meta.created_by or "").strip():
        errs.append("metadata.created_by is empty")
    if not (meta.notes or "").strip():
        errs.append("metadata.notes is empty")
    if not meta.enabled_feature_families:
        errs.append("metadata.enabled_feature_families must be non-empty")
    if errs:
        raise ValueError("canonical metadata incomplete: " + "; ".join(errs))


def validate_production_canonical_metadata(meta: "CanonicalMetadata", *, execution_mode: str) -> None:
    """Stricter rules for live trading or when ``NM_CANONICAL_CONFIG_STRICT`` is set."""
    if not canonical_config_strict_enabled(execution_mode):
        return
    es = (meta.environment_scope or "").strip().lower()
    if es == "unspecified":
        raise ValueError(
            "canonical metadata rejected for production: metadata.environment_scope must not be "
            "'unspecified' when NM_CANONICAL_CONFIG_STRICT is set or execution_mode is live "
            "(set apex_canonical.metadata.environment_scope to research, simulation, shadow, or live)"
        )


def validate_canonical_runtime_metadata(
    meta: "CanonicalMetadata",
    *,
    execution_mode: str,
) -> None:
    """Run completeness + optional production checks."""
    validate_canonical_metadata_complete(meta)
    validate_production_canonical_metadata(meta, execution_mode=execution_mode)
