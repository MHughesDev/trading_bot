"""APEX release gating and promotion lifecycle (FB-CAN-011).

Implements logical gates from
``docs/Human Provided Specs/new_specs/canonical/APEX_Config_Management_and_Release_Gating_Spec_v1_0.md``:
immutable release candidate records, environment progression research→simulation→shadow→live,
rollback metadata, and evidence-package checks before promotion.

This module is **operator tooling**: it does not auto-promote MLflow models (see CI policy).
"""

from __future__ import annotations

import json
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field

from orchestration.fault_injection_profiles import fault_stress_evidence_satisfied

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
    """Immediate rollback pointer (spec §10)."""

    model_config = ConfigDict(extra="ignore")

    target_config_version: str | None = None
    target_logic_version: str | None = None
    target_model_family_ref: str | None = None
    instructions: str = ""
    trigger_conditions: str = ""
    rollback_owner: str = ""


class EvidencePackage(BaseModel):
    """Material-release evidence bundle (spec §8)."""

    model_config = ConfigDict(extra="ignore")

    version_identifiers: dict[str, str] = Field(default_factory=dict)
    domains_changed: list[str] = Field(default_factory=list)
    replay_summary: str = ""
    replay_run_ids: list[str] = Field(default_factory=list)
    scenario_stress_summary: str = ""
    shadow_comparison_summary: str = ""
    expected_benefits: str = ""
    known_risks: str = ""
    owner_approval_present: bool = False
    owner_approver: str = ""
    unit_tests_passed: bool | None = None
    scenario_tests_passed: bool | None = None
    replay_regression_passed: bool | None = None
    shadow_divergence_reviewed: bool | None = None
    holdout_evidence_present: bool | None = None  # model-family gate
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
    """Outcome of :func:`evaluate_promotion_gates`."""

    allowed: bool
    target_environment: ReleaseEnvironment
    reasons: list[str] = Field(default_factory=list)
    blocked_gates: list[str] = Field(default_factory=list)


def _needs_strong_evidence(severity: ReleaseSeverity) -> bool:
    return severity == "major"


def evaluate_promotion_gates(
    candidate: ReleaseCandidate,
    *,
    target_environment: ReleaseEnvironment,
) -> PromotionGateResult:
    """
    Evaluate mandatory gates before advancing ``candidate`` toward ``target_environment``.

    Rules are a **minimal** encoding of spec §7: missing evidence → not allowed.
    """
    reasons: list[str] = []
    blocked: list[str] = []

    ev = candidate.evidence
    rb = candidate.rollback

    # --- Universal: rollback target (spec §7.1) ---
    has_rollback = bool(
        (rb.target_config_version and rb.target_config_version.strip())
        or (rb.target_logic_version and rb.target_logic_version.strip())
        or (rb.instructions and rb.instructions.strip())
    )
    if not has_rollback:
        blocked.append("rollback_target_defined")
        reasons.append("rollback target must include config/logic version or instructions")

    # --- Owner ---
    if not (candidate.owner and candidate.owner.strip()):
        blocked.append("owner_present")
        reasons.append("owner must be set")

    # --- Schema: pydantic already validated; evidence version map ---
    if not ev.version_identifiers:
        blocked.append("version_identifiers")
        reasons.append("evidence.version_identifiers should name config/logic (and model) versions")

    # Environment-specific gates
    if target_environment == "simulation":
        if candidate.current_stage not in ("draft", "reviewed"):
            reasons.append("note: current_stage is not draft/reviewed before simulation")
        if not ev.replay_summary.strip() and not ev.replay_run_ids:
            blocked.append("replay_evidence")
            reasons.append("simulation requires replay evidence (summary or run ids)")

    if target_environment == "shadow":
        if not ev.replay_summary.strip() and not ev.replay_run_ids:
            blocked.append("replay_evidence")
        if not ev.scenario_stress_summary.strip():
            blocked.append("scenario_stress")
            reasons.append("shadow promotion requires scenario stress summary")
        if candidate.evidence.owner_approval_present is not True:
            blocked.append("owner_approval")
            reasons.append("shadow path requires owner approval flag")
        if not fault_stress_evidence_satisfied(
            fault_stress_run_ids=ev.fault_stress_run_ids,
            fault_profile_ids_satisfied=ev.fault_profile_ids_satisfied,
        ):
            blocked.append("fault_stress_evidence")
            reasons.append(
                "shadow requires fault stress replay ids and all canonical fault profile ids (FB-CAN-037)"
            )

    if target_environment == "live":
        if not ev.owner_approval_present:
            blocked.append("owner_approval")
        if not has_rollback:
            pass  # already blocked
        if not ev.replay_summary.strip() and not ev.replay_run_ids:
            blocked.append("replay_evidence")
        if not ev.scenario_stress_summary.strip():
            blocked.append("scenario_stress")
        if ev.shadow_divergence_reviewed is not True:
            blocked.append("shadow_divergence_reviewed")
            reasons.append("live requires shadow divergence reviewed (spec §7.2–9.4)")
        if not fault_stress_evidence_satisfied(
            fault_stress_run_ids=ev.fault_stress_run_ids,
            fault_profile_ids_satisfied=ev.fault_profile_ids_satisfied,
        ):
            blocked.append("fault_stress_evidence")
            reasons.append(
                "live requires fault stress replay evidence and full canonical fault profile coverage (FB-CAN-037)"
            )
        # Logic gates
        if candidate.kind in ("logic", "combined"):
            if ev.unit_tests_passed is not True:
                blocked.append("unit_tests")
            if ev.scenario_tests_passed is not True:
                blocked.append("scenario_tests")
            if ev.replay_regression_passed is not True:
                blocked.append("replay_regression")
            if ev.live_replay_equivalence_passed is not True:
                blocked.append("live_replay_equivalence")
                reasons.append("live requires live–replay deterministic equivalence (FB-CAN-030)")
        if candidate.kind == "combined":
            if ev.live_replay_equivalence_passed is not True:
                blocked.append("live_replay_equivalence")
                reasons.append("live requires live–replay equivalence for combined logic+config releases")
        # Model family gates
        if candidate.kind in ("model_family", "combined"):
            if ev.holdout_evidence_present is not True:
                blocked.append("holdout_evidence")
            if ev.replay_regression_passed is not True:
                blocked.append("replay_regression_model")

    if _needs_strong_evidence(candidate.severity):
        if not ev.known_risks.strip():
            blocked.append("known_risks_documented")
            reasons.append("major severity requires known_risks in evidence package")

    allowed = len(blocked) == 0
    if allowed:
        reasons.append(f"gates passed for target_environment={target_environment!r}")

    return PromotionGateResult(
        allowed=allowed,
        target_environment=target_environment,
        reasons=reasons,
        blocked_gates=sorted(set(blocked)),
    )


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
