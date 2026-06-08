"""Canonical release-object schema (FB-CAN-011, FB-CAN-051).

Aligned with
``APEX_Config_Management_and_Release_Gating_Spec_v1_0.md`` §3: config, logic,
model-family, feature-family, and combined releases — each with owner, rationale,
evidence package, and rollback metadata.

Persistence helpers read/write ``models/registry/release_ledger.json`` by default.
"""

from __future__ import annotations

import json
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field

ReleaseObjectKind = Literal["config", "logic", "model_family", "feature_family", "combined"]
ReleaseEnvironment = Literal["research", "simulation", "shadow", "live"]
ConfigLifecycleStage = Literal[
    "draft",
    "reviewed",
    "simulated",
    "shadowed",
    "approved_for_live",
    "active_live",
    "retired",
    "rolled_back",
]
ReleaseSeverity = Literal["minor", "moderate", "major"]


class RollbackTarget(BaseModel):
    """Immediate rollback pointer (spec §10). FB-CAN-051: feature-family rollback list."""

    model_config = ConfigDict(extra="ignore")

    target_config_version: str | None = None
    target_logic_version: str | None = None
    target_model_family_ref: str | None = None
    target_feature_family_refs: list[str] = Field(
        default_factory=list,
        description="Prior enabled feature-family ids or labels to restore on rollback.",
    )
    instructions: str = ""
    trigger_conditions: str = ""
    rollback_owner: str = ""


class EvidencePackage(BaseModel):
    """Material-release evidence bundle (spec §8).

    **FB-CAN-066** — canonical promotion evidence schema: links replay runs, scenario tests,
    shadow narrative, key metric snapshots, failure-mode documentation, and ties to rollback
    (rollback lives on :class:`RollbackTarget`). Completeness is enforced in
    :func:`orchestration.release_gating.evaluate_promotion_gates` for simulation+ targets.
    """

    model_config = ConfigDict(extra="ignore")

    schema_version: int = Field(
        default=1,
        ge=1,
        description="Evidence package schema revision (FB-CAN-066).",
    )
    version_identifiers: dict[str, str] = Field(default_factory=dict)
    domains_changed: list[str] = Field(default_factory=list)
    replay_summary: str = ""
    replay_run_ids: list[str] = Field(default_factory=list)
    scenario_stress_summary: str = ""
    scenario_test_run_ids: list[str] = Field(
        default_factory=list,
        description="Scenario / stress replay run ids (distinct from fault_stress_run_ids).",
    )
    shadow_comparison_summary: str = ""
    expected_benefits: str = ""
    known_risks: str = ""
    key_metrics: dict[str, float] = Field(
        default_factory=dict,
        description="Named metric snapshots for promotion review (FB-CAN-066).",
    )
    failure_modes_documented: str = Field(
        default="",
        description="Explicit failure modes / mitigations; complements known_risks and experiments.",
    )
    owner_approval_present: bool = False
    owner_approver: str = ""
    unit_tests_passed: bool | None = None
    scenario_tests_passed: bool | None = None
    replay_regression_passed: bool | None = None
    shadow_divergence_reviewed: bool | None = None
    holdout_evidence_present: bool | None = None
    feature_family_replay_passed: bool | None = Field(
        default=None,
        description="True when replay evidence covers the toggled feature-family slice (FB-CAN-051).",
    )
    live_replay_equivalence_passed: bool | None = Field(
        default=None,
        description="True when live–replay equivalence harness reports no divergence (FB-CAN-030).",
    )
    live_replay_equivalence_report: dict[str, Any] | None = Field(
        default=None,
        description="Optional JSON snapshot from LiveReplayEquivalenceReport.",
    )
    fault_stress_run_ids: list[str] = Field(
        default_factory=list,
        description="Replay run ids under canonical fault profiles (FB-CAN-037).",
    )
    fault_profile_ids_satisfied: list[str] = Field(
        default_factory=list,
        description="Canonical fault profile ids exercised for those runs.",
    )
    shadow_comparison_report: dict[str, Any] | None = Field(
        default=None,
        description="Structured shadow vs baseline replay comparison (FB-CAN-038).",
    )
    shadow_comparison_passed: bool | None = Field(
        default=None,
        description="True when comparison is within thresholds and probation (FB-CAN-038).",
    )


class ReleaseCandidate(BaseModel):
    """Immutable-style release record; bump ``release_id`` for new versions."""

    model_config = ConfigDict(extra="ignore")

    release_id: str
    kind: ReleaseObjectKind
    owner: str
    rationale: str = ""
    release_notes: str = ""
    config_version: str
    logic_version: str | None = None
    model_family_ref: str | None = None
    feature_family_refs: list[str] = Field(default_factory=list)
    severity: ReleaseSeverity = "moderate"
    current_stage: ConfigLifecycleStage = "draft"
    environment: ReleaseEnvironment = "research"
    evidence: EvidencePackage = Field(default_factory=EvidencePackage)
    rollback: RollbackTarget = Field(default_factory=RollbackTarget)
    created_at: str = Field(
        default_factory=lambda: datetime.now(UTC).replace(microsecond=0).isoformat()
    )
    approved_at: str | None = None
    linked_experiment_ids: list[str] = Field(default_factory=list)

    def to_serializable_dict(self) -> dict[str, Any]:
        return self.model_dump(mode="json")


class PromotionGateResult(BaseModel):
    """Outcome of :func:`orchestration.release_gating.evaluate_promotion_gates`."""

    allowed: bool
    target_environment: ReleaseEnvironment
    reasons: list[str] = Field(default_factory=list)
    blocked_gates: list[str] = Field(default_factory=list)


class ReleaseLedger(BaseModel):
    """Append-only friendly list of candidates (JSON file)."""

    model_config = ConfigDict(extra="ignore")

    schema_version: int = 1
    updated_at: str = Field(
        default_factory=lambda: datetime.now(UTC).replace(microsecond=0).isoformat()
    )
    candidates: list[ReleaseCandidate] = Field(default_factory=list)


def default_release_ledger_path() -> Path:
    return Path("models") / "registry" / "release_ledger.json"


def read_release_ledger(path: str | Path | None = None) -> ReleaseLedger | None:
    p = Path(path) if path is not None else default_release_ledger_path()
    if not p.is_file():
        return None
    try:
        raw = json.loads(p.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(raw, dict):
        return None
    return ReleaseLedger.model_validate(raw)


def write_release_ledger(ledger: ReleaseLedger, path: str | Path | None = None) -> Path:
    p = Path(path) if path is not None else default_release_ledger_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    body = ledger.model_dump(mode="json")
    body["updated_at"] = datetime.now(UTC).replace(microsecond=0).isoformat()
    p.write_text(json.dumps(body, indent=2), encoding="utf-8")
    return p


__all__ = [
    "ConfigLifecycleStage",
    "EvidencePackage",
    "PromotionGateResult",
    "ReleaseCandidate",
    "ReleaseEnvironment",
    "ReleaseLedger",
    "ReleaseObjectKind",
    "ReleaseSeverity",
    "RollbackTarget",
    "default_release_ledger_path",
    "read_release_ledger",
    "write_release_ledger",
]
