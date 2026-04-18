"""APEX release gating and promotion lifecycle (FB-CAN-011, FB-CAN-051, FB-CAN-052).

Schema lives in :mod:`app.contracts.release_objects`; this module evaluates
promotion gates from ``APEX_Config_Management_and_Release_Gating_Spec_v1_0.md``.

**FB-CAN-052:** ``ReleaseCandidate.environment`` must equal the **immediate predecessor**
in research → simulation → shadow → live before promoting to the next stage (no skips).

**FB-CAN-053:** Rollback playbook fields (owner, instructions, triggers) required for
promotion beyond **research**; structural validation on rollback version refs.

**FB-CAN-054:** Linked experiments must satisfy predeclared metrics and failure-mode
documentation before release promotion (lazy import avoids circular deps with
``models.registry.experiment_registry``).
"""

from __future__ import annotations

from pathlib import Path

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
from orchestration.rollback_validation import (
    validate_rollback_playbook,
    validate_rollback_target_references,
)

# Spec §4 — ordered environments (FB-CAN-052).
_ENVIRONMENT_ORDER: tuple[ReleaseEnvironment, ...] = ("research", "simulation", "shadow", "live")


def required_environment_before(target_environment: ReleaseEnvironment) -> ReleaseEnvironment | None:
    """Environment the candidate must occupy before a one-step promotion to ``target_environment``."""
    try:
        i = _ENVIRONMENT_ORDER.index(target_environment)
    except ValueError:
        return None
    if i == 0:
        return None
    return _ENVIRONMENT_ORDER[i - 1]


def _environment_progression_ok(
    candidate: ReleaseCandidate,
    target_environment: ReleaseEnvironment,
) -> tuple[bool, str]:
    req = required_environment_before(target_environment)
    if req is None:
        return True, ""
    if candidate.environment != req:
        return (
            False,
            f"candidate.environment must be {req!r} before promoting to {target_environment!r}; "
            f"got {candidate.environment!r} (no implicit cross-stage promotion; FB-CAN-052)",
        )
    return True, ""


def validate_rollback_for_promotion(
    rollback: RollbackTarget,
    *,
    target_environment: ReleaseEnvironment,
) -> tuple[bool, list[str]]:
    """Structural refs + operator playbook (FB-CAN-053)."""
    ok_a, ra = validate_rollback_target_references(rollback)
    ok_b, rb_reasons = validate_rollback_playbook(rollback, target_environment=target_environment)
    return ok_a and ok_b, ra + rb_reasons


def validate_linked_experiments_for_promotion(
    candidate: ReleaseCandidate,
    *,
    target_environment: ReleaseEnvironment,
    experiment_registry_path: str | Path | None = None,
) -> tuple[bool, list[str]]:
    """
    FB-CAN-054: each linked experiment must pass registry completeness checks
    (predeclared metrics, success decision, failure modes) before promotion to
    simulation or beyond.
    """
    if target_environment == "research":
        return True, []
    ids = [x.strip() for x in candidate.linked_experiment_ids if x and x.strip()]
    if not ids:
        return True, []

    # Lazy import: ``experiment_registry`` imports this module at top level.
    from models.registry.experiment_registry import (  # noqa: PLC0415
        get_experiment_by_id,
        read_experiment_registry,
    )

    reg = read_experiment_registry(experiment_registry_path)
    if reg is None:
        return False, [
            "linked_experiment_ids is non-empty but experiment registry file is missing or unreadable"
        ]

    reasons: list[str] = []
    from models.registry.experiment_validation import (  # noqa: PLC0415
        validate_experiment_promotion_readiness,
    )

    for eid in ids:
        rec = get_experiment_by_id(reg, eid)
        if rec is None:
            reasons.append(f"linked experiment {eid!r} not found in experiment registry")
            continue
        ok, msgs = validate_experiment_promotion_readiness(rec)
        if not ok:
            for m in msgs:
                reasons.append(f"experiment {eid}: {m}")
    return len(reasons) == 0, reasons


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
    "required_environment_before",
    "validate_rollback_for_promotion",
    "validate_linked_experiments_for_promotion",
    "write_release_ledger",
]


def _needs_strong_evidence(severity: str) -> bool:
    return severity == "major"


def evaluate_promotion_gates(
    candidate: ReleaseCandidate,
    *,
    target_environment: ReleaseEnvironment,
    experiment_registry_path: str | Path | None = None,
) -> PromotionGateResult:
    """
    Evaluate mandatory gates before advancing ``candidate`` toward ``target_environment``.

    Rules are a **minimal** encoding of spec §7: missing evidence → not allowed.

    ``experiment_registry_path`` (FB-CAN-054): optional override for the experiment registry JSON
    when validating ``linked_experiment_ids`` (defaults to on-disk path).
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
    elif target_environment != "research":
        ok_rb, rb_msgs = validate_rollback_for_promotion(rb, target_environment=target_environment)
        if not ok_rb:
            blocked.append("rollback_playbook")
            reasons.extend(rb_msgs)

    # --- Linked experiments (FB-CAN-054) ---
    ok_exp, exp_msgs = validate_linked_experiments_for_promotion(
        candidate,
        target_environment=target_environment,
        experiment_registry_path=experiment_registry_path,
    )
    if not ok_exp:
        blocked.append("linked_experiments_complete")
        reasons.extend(exp_msgs)

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

    # --- Explicit environment-stage progression (spec §4; FB-CAN-052) ---
    ok_env, env_msg = _environment_progression_ok(candidate, target_environment)
    if not ok_env:
        blocked.append("environment_stage_order")
        reasons.append(env_msg)

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

    result = PromotionGateResult(
        allowed=allowed,
        target_environment=target_environment,
        reasons=reasons,
        blocked_gates=sorted(set(blocked)),
    )
    try:
        from observability.governance_metrics import record_promotion_gate_result  # noqa: PLC0415

        record_promotion_gate_result(result, kind=str(candidate.kind))
    except Exception:
        pass
    return result
