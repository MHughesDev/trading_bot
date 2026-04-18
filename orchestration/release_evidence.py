"""APEX release evidence bundles and canonical config fingerprints (FB-CAN-026).

Builds JSON-serializable artifacts for promotion: config fingerprint, optional diff vs a
baseline YAML, replay/shadow run references, and rollback metadata (see APEX Config
Management spec §8).
"""

from __future__ import annotations

import hashlib
import json
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import yaml
from pydantic import BaseModel, ConfigDict, Field

from app.config.canonical_config import CanonicalRuntimeConfig, resolve_canonical_config
from app.config.settings import AppSettings, load_settings
from orchestration.release_gating import RollbackTarget


class ReleaseEvidenceBundle(BaseModel):
    """Operator-facing evidence package for CI and control-plane (spec §8 + FB-CAN-026)."""

    model_config = ConfigDict(extra="ignore")

    schema_version: int = 1
    generated_at: str = Field(
        default_factory=lambda: datetime.now(UTC).replace(microsecond=0).isoformat()
    )
    config_version: str
    logic_version: str | None = None
    canonical_config_fingerprint: str
    canonical_diff_vs_baseline: dict[str, Any] | None = None
    replay_run_ids: list[str] = Field(default_factory=list)
    shadow_run_ids: list[str] = Field(default_factory=list)
    rollback: RollbackTarget = Field(default_factory=RollbackTarget)


def canonical_runtime_fingerprint(cfg: CanonicalRuntimeConfig) -> str:
    """Stable SHA-256 over the canonical JSON snapshot (sorted keys)."""
    payload = json.dumps(cfg.model_dump(mode="json"), sort_keys=True, default=str)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def _diff_any(a: Any, b: Any, path: str) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    if type(a) is not type(b):
        out.append({"path": path, "old": a, "new": b, "kind": "type_mismatch"})
        return out
    if isinstance(a, dict) and isinstance(b, dict):
        keys = sorted(set(a.keys()) | set(b.keys()))
        for k in keys:
            p = f"{path}.{k}" if path else k
            if k not in a:
                out.append({"path": p, "old": None, "new": b[k], "kind": "added"})
            elif k not in b:
                out.append({"path": p, "old": a[k], "new": None, "kind": "removed"})
            else:
                out.extend(_diff_any(a[k], b[k], p))
        return out
    if isinstance(a, list) and isinstance(b, list):
        if a != b:
            out.append({"path": path or "<root>", "old": a, "new": b, "kind": "list_changed"})
        return out
    if a != b:
        out.append({"path": path or "<root>", "old": a, "new": b, "kind": "changed"})
    return out


def diff_canonical_runtime(
    baseline: CanonicalRuntimeConfig,
    current: CanonicalRuntimeConfig,
) -> dict[str, Any]:
    """Structured diff between two resolved canonical bundles (domain-level)."""
    bd = baseline.model_dump(mode="json")
    cd = current.model_dump(mode="json")
    changes = _diff_any(bd, cd, "")
    return {
        "baseline_config_version": baseline.metadata.config_version,
        "current_config_version": current.metadata.config_version,
        "baseline_logic_version": baseline.metadata.logic_version,
        "current_logic_version": current.metadata.logic_version,
        "change_count": len(changes),
        "changes": changes,
    }


def resolve_canonical_from_yaml_file(path: Path | str) -> CanonicalRuntimeConfig:
    """Load YAML using the same merge rules as runtime (full config document)."""
    p = Path(path)
    raw = yaml.safe_load(p.read_text(encoding="utf-8"))
    if not isinstance(raw, dict):
        raise ValueError(f"YAML root must be a mapping: {p}")
    settings = AppSettings()
    return resolve_canonical_config(settings, raw)


def resolve_canonical_from_yaml_text(yaml_text: str) -> CanonicalRuntimeConfig:
    """Parse YAML text (e.g. pasted ``default.yaml``) with the same merge rules as runtime."""
    raw = yaml.safe_load(yaml_text)
    if not isinstance(raw, dict):
        raise ValueError("YAML root must be a mapping")
    settings = AppSettings()
    return resolve_canonical_config(settings, raw)


def build_release_evidence_bundle(
    *,
    settings: AppSettings | None = None,
    baseline_yaml_path: Path | str | None = None,
    baseline_yaml_text: str | None = None,
    replay_run_ids: list[str] | None = None,
    shadow_run_ids: list[str] | None = None,
    rollback: RollbackTarget | None = None,
) -> ReleaseEvidenceBundle:
    """Compose a evidence bundle from live settings and optional baseline file."""
    s = settings or load_settings()
    current = s.canonical
    fp = canonical_runtime_fingerprint(current)
    diff: dict[str, Any] | None = None
    baseline: CanonicalRuntimeConfig | None = None
    if baseline_yaml_text is not None and baseline_yaml_text.strip():
        baseline = resolve_canonical_from_yaml_text(baseline_yaml_text)
    elif baseline_yaml_path is not None:
        baseline = resolve_canonical_from_yaml_file(baseline_yaml_path)
    if baseline is not None:
        diff = diff_canonical_runtime(baseline, current)

    return ReleaseEvidenceBundle(
        config_version=current.metadata.config_version,
        logic_version=current.metadata.logic_version,
        canonical_config_fingerprint=fp,
        canonical_diff_vs_baseline=diff,
        replay_run_ids=list(replay_run_ids or []),
        shadow_run_ids=list(shadow_run_ids or []),
        rollback=rollback or RollbackTarget(),
    )
