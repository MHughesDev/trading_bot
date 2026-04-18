"""Canonical evidence package completeness (FB-CAN-066).

Used by :func:`orchestration.release_gating.evaluate_promotion_gates` to block promotion
when required links (replay, scenario runs, shadow narrative, metrics, failure modes) are missing.
"""

from __future__ import annotations

from app.contracts.release_objects import EvidencePackage, ReleaseCandidate, ReleaseEnvironment


def evidence_completeness_blocked_gates(
    candidate: ReleaseCandidate,
    ev: EvidencePackage,
    *,
    target_environment: ReleaseEnvironment,
) -> tuple[list[str], list[str]]:
    """
    Return (blocked_gate_ids, human_reasons) for FB-CAN-066 schema completeness.

    Rules apply when ``target_environment`` is **simulation**, **shadow**, or **live**.
    Research-only releases are not subject to these completeness checks.
    """
    if target_environment == "research":
        return [], []

    blocked: list[str] = []
    reasons: list[str] = []

    if ev.schema_version < 1:
        blocked.append("evidence_schema_version")
        reasons.append("evidence.schema_version must be >= 1 (FB-CAN-066)")

    if not ev.key_metrics:
        blocked.append("key_metrics")
        reasons.append(
            "evidence.key_metrics must include at least one named metric snapshot for promotion "
            "beyond research (FB-CAN-066)"
        )

    has_failure_modes = bool(ev.failure_modes_documented.strip())
    has_experiments = bool([x for x in candidate.linked_experiment_ids if x and str(x).strip()])
    if not has_failure_modes and not has_experiments:
        blocked.append("failure_modes")
        reasons.append(
            "set evidence.failure_modes_documented or link experiments with documented failure modes "
            "(FB-CAN-066)"
        )

    if not ev.scenario_test_run_ids:
        blocked.append("scenario_test_runs")
        reasons.append(
            "evidence.scenario_test_run_ids must list at least one scenario/replay run id "
            "(FB-CAN-066)"
        )

    if target_environment in ("shadow", "live"):
        if not ev.shadow_comparison_summary.strip():
            blocked.append("shadow_comparison_summary")
            reasons.append(
                "evidence.shadow_comparison_summary must document the shadow comparison outcome "
                "(FB-CAN-066)"
            )

    return blocked, reasons


__all__ = ["evidence_completeness_blocked_gates"]
