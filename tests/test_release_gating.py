from __future__ import annotations

from orchestration.fault_injection_profiles import list_canonical_fault_profile_ids
from orchestration.release_gating import (
    EvidencePackage,
    ReleaseCandidate,
    ReleaseLedger,
    RollbackTarget,
    evaluate_promotion_gates,
    read_release_ledger,
    required_environment_before,
    write_release_ledger,
)

_ALL_FAULT_IDS = list(list_canonical_fault_profile_ids())


def _fault_stress_fields() -> dict:
    return {
        "fault_stress_run_ids": ["ci-fault-suite"],
        "fault_profile_ids_satisfied": _ALL_FAULT_IDS,
    }


def _env_shadow() -> dict:
    """FB-CAN-052: live promotion requires prior stage == shadow."""
    return {"environment": "shadow"}


def _minimal_evidence_live() -> EvidencePackage:
    return EvidencePackage(
        version_identifiers={"config": "1.0.0", "logic": "1.0.0"},
        replay_summary="ok",
        replay_run_ids=["r1"],
        scenario_stress_summary="ok",
        known_risks="none",
        owner_approval_present=True,
        shadow_divergence_reviewed=True,
        shadow_comparison_passed=True,
        **_fault_stress_fields(),
    )


def test_gates_fail_without_rollback():
    c = ReleaseCandidate(
        release_id="rel-test-1",
        kind="config",
        owner="ops",
        config_version="1.0.0",
        **_env_shadow(),
        evidence=_minimal_evidence_live(),
        rollback=RollbackTarget(),
    )
    r = evaluate_promotion_gates(c, target_environment="live")
    assert r.allowed is False
    assert "rollback_target_defined" in r.blocked_gates


def test_gates_pass_config_live():
    c = ReleaseCandidate(
        release_id="rel-test-2",
        kind="config",
        owner="ops",
        config_version="1.0.0",
        **_env_shadow(),
        evidence=_minimal_evidence_live(),
        rollback=RollbackTarget(target_config_version="0.9.0", instructions="revert yaml"),
    )
    r = evaluate_promotion_gates(c, target_environment="live")
    assert r.allowed is True


def test_combined_live_requires_live_replay_equivalence():
    c = ReleaseCandidate(
        release_id="rel-comb",
        kind="combined",
        owner="ops",
        config_version="1.0.0",
        logic_version="2.0.0",
        **_env_shadow(),
        evidence=EvidencePackage(
            version_identifiers={"config": "1.0.0", "logic": "2.0.0"},
            replay_summary="ok",
            scenario_stress_summary="ok",
            known_risks="reviewed",
            owner_approval_present=True,
            shadow_divergence_reviewed=True,
            unit_tests_passed=True,
            scenario_tests_passed=True,
            replay_regression_passed=True,
            live_replay_equivalence_passed=None,
            shadow_comparison_passed=True,
            **_fault_stress_fields(),
        ),
        rollback=RollbackTarget(target_config_version="0.9.0"),
    )
    r = evaluate_promotion_gates(c, target_environment="live")
    assert r.allowed is False
    assert "live_replay_equivalence" in r.blocked_gates


def test_logic_live_requires_live_replay_equivalence():
    c = ReleaseCandidate(
        release_id="rel-test-equiv",
        kind="logic",
        owner="ops",
        config_version="1.0.0",
        logic_version="2.0.0",
        **_env_shadow(),
        evidence=EvidencePackage(
            version_identifiers={"config": "1.0.0", "logic": "2.0.0"},
            replay_summary="ok",
            scenario_stress_summary="ok",
            known_risks="reviewed",
            owner_approval_present=True,
            shadow_divergence_reviewed=True,
            unit_tests_passed=True,
            scenario_tests_passed=True,
            replay_regression_passed=True,
            live_replay_equivalence_passed=False,
            shadow_comparison_passed=True,
            **_fault_stress_fields(),
        ),
        rollback=RollbackTarget(target_logic_version="1.0.0"),
    )
    r = evaluate_promotion_gates(c, target_environment="live")
    assert r.allowed is False
    assert "live_replay_equivalence" in r.blocked_gates


def test_logic_live_requires_test_flags():
    c = ReleaseCandidate(
        release_id="rel-test-3",
        kind="logic",
        owner="ops",
        config_version="1.0.0",
        logic_version="2.0.0",
        **_env_shadow(),
        evidence=EvidencePackage(
            version_identifiers={"config": "1.0.0", "logic": "2.0.0"},
            replay_summary="ok",
            scenario_stress_summary="ok",
            known_risks="reviewed",
            owner_approval_present=True,
            shadow_divergence_reviewed=True,
            **_fault_stress_fields(),
        ),
        rollback=RollbackTarget(target_logic_version="1.0.0"),
    )
    r = evaluate_promotion_gates(c, target_environment="live")
    assert r.allowed is False
    assert "unit_tests" in r.blocked_gates

    ev2 = c.evidence.model_copy(
        update={
            "unit_tests_passed": True,
            "scenario_tests_passed": True,
            "replay_regression_passed": True,
            "live_replay_equivalence_passed": True,
            "shadow_comparison_passed": True,
            **_fault_stress_fields(),
        }
    )
    c2 = c.model_copy(update={"evidence": ev2})
    r2 = evaluate_promotion_gates(c2, target_environment="live")
    assert r2.allowed is True


def test_live_blocked_when_not_shadow_stage():
    c = ReleaseCandidate(
        release_id="rel-skip",
        kind="config",
        owner="ops",
        config_version="1.0.0",
        environment="research",
        evidence=_minimal_evidence_live(),
        rollback=RollbackTarget(target_config_version="0.9.0", instructions="revert yaml"),
    )
    r = evaluate_promotion_gates(c, target_environment="live")
    assert r.allowed is False
    assert "environment_stage_order" in r.blocked_gates


def test_simulation_requires_prior_research_environment():
    c = ReleaseCandidate(
        release_id="rel-sim",
        kind="config",
        owner="ops",
        config_version="1.0.0",
        environment="shadow",
        evidence=EvidencePackage(
            version_identifiers={"config": "1.0.0"},
            replay_summary="ok",
            replay_run_ids=["r1"],
        ),
        rollback=RollbackTarget(target_config_version="0.9.0"),
    )
    r = evaluate_promotion_gates(c, target_environment="simulation")
    assert r.allowed is False
    assert "environment_stage_order" in r.blocked_gates

    c2 = c.model_copy(update={"environment": "research"})
    r2 = evaluate_promotion_gates(c2, target_environment="simulation")
    assert r2.allowed is True


def test_required_environment_before_live_is_shadow():
    assert required_environment_before("live") == "shadow"


def test_live_blocked_without_shadow_comparison():
    ev = _minimal_evidence_live().model_copy(update={"shadow_comparison_passed": False})
    c = ReleaseCandidate(
        release_id="rel-no-shadow",
        kind="config",
        owner="ops",
        config_version="1.0.0",
        **_env_shadow(),
        evidence=ev,
        rollback=RollbackTarget(target_config_version="0.9.0", instructions="revert"),
    )
    r = evaluate_promotion_gates(c, target_environment="live")
    assert r.allowed is False
    assert "shadow_comparison" in r.blocked_gates


def test_live_blocked_without_fault_stress_evidence():
    ev = _minimal_evidence_live().model_copy(
        update={"fault_stress_run_ids": [], "fault_profile_ids_satisfied": []}
    )
    c = ReleaseCandidate(
        release_id="rel-no-fault",
        kind="config",
        owner="ops",
        config_version="1.0.0",
        **_env_shadow(),
        evidence=ev,
        rollback=RollbackTarget(target_config_version="0.9.0", instructions="revert"),
    )
    r = evaluate_promotion_gates(c, target_environment="live")
    assert r.allowed is False
    assert "fault_stress_evidence" in r.blocked_gates


def test_feature_family_live_requires_replay_flag():
    ev = _minimal_evidence_live().model_copy(update={"feature_family_replay_passed": False})
    c = ReleaseCandidate(
        release_id="rel-ff",
        kind="feature_family",
        owner="ops",
        config_version="1.0.0",
        **_env_shadow(),
        feature_family_refs=["funding"],
        evidence=ev,
        rollback=RollbackTarget(target_config_version="0.9.0", target_feature_family_refs=["funding"]),
    )
    r = evaluate_promotion_gates(c, target_environment="live")
    assert r.allowed is False
    assert "feature_family_replay" in r.blocked_gates

    ev2 = ev.model_copy(update={"feature_family_replay_passed": True})
    c2 = c.model_copy(update={"evidence": ev2})
    r2 = evaluate_promotion_gates(c2, target_environment="live")
    assert r2.allowed is True


def test_release_ledger_roundtrip(tmp_path):
    path = tmp_path / "ledger.json"
    led = ReleaseLedger(
        candidates=[
            ReleaseCandidate(
                release_id="x",
                kind="config",
                owner="a",
                config_version="1",
                evidence=EvidencePackage(version_identifiers={"config": "1"}),
                rollback=RollbackTarget(target_config_version="0"),
            )
        ]
    )
    write_release_ledger(led, path)
    back = read_release_ledger(path)
    assert back is not None
    assert len(back.candidates) == 1
    assert back.candidates[0].release_id == "x"
