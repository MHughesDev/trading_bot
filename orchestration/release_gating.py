"""APEX release gating and promotion lifecycle (FB-CAN-011, FB-CAN-051).

Schema lives in :mod:`app.contracts.release_objects`; this module evaluates
promotion gates from ``APEX_Config_Management_and_Release_Gating_Spec_v1_0.md``.
"""

from __future__ import annotations

from app.contracts.release_objects import (
    ConfigLifecycleStage,
    EvidencePackage,
    PromotionGateResult,
    ReleaseCandidate,
    ReleaseEnvironment,
    ReleaseLedger,
    ReleaseObjectKind,
    ReleaseSeverity,
    RollbackTarget,
    default_release_ledger_path,
    read_release_ledger,
    write_release_ledger,
)
from orchestration.fault_injection_profiles import fault_stress_evidence_satisfied

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
    "evaluate_promotion_gates",
    "read_release_ledger",
    "write_release_ledger",
]


def _needs_strong_evidence(severity: str) -> bool:
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
        or (rb.target_model_family_ref and rb.target_model_family_ref.strip())
        or (rb.target_feature_family_refs and len(rb.target_feature_family_refs) > 0)
        or (rb.instructions and rb.instructions.strip())
    )
    if not has_rollback:
        blocked.append("rollback_target_defined")
        reasons.append(
            "rollback target must include config/logic/model ref, feature-family list, or instructions"
        )

    # --- Owner ---
    if not (candidate.owner and candidate.owner.strip()):
        blocked.append("owner_present")
        reasons.append("owner must be set")

    # --- Schema: pydantic already validated; evidence version map ---
    if not ev.version_identifiers:
        blocked.append("version_identifiers")
        reasons.append("evidence.version_identifiers should name config/logic (and model) versions")

    # Feature-family release: must name families
    if candidate.kind == "feature_family":
        if not candidate.feature_family_refs:
            blocked.append("feature_family_refs")
            reasons.append("feature_family release requires non-empty feature_family_refs")

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
        if ev.shadow_comparison_passed is not True:
            blocked.append("shadow_comparison")
            reasons.append("shadow requires shadow_comparison_passed (FB-CAN-038)")

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
        if ev.shadow_comparison_passed is not True:
            blocked.append("shadow_comparison")
            reasons.append("live requires shadow replay comparison within thresholds (FB-CAN-038)")
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
        # Feature-family gates (spec §7 — replay + shadow; FB-CAN-051)
        if candidate.kind == "feature_family":
            if ev.feature_family_replay_passed is not True:
                blocked.append("feature_family_replay")
                reasons.append("live feature-family release requires feature_family_replay_passed")

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
