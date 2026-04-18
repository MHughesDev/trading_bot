"""FB-CAN-066: canonical evidence package completeness."""

from __future__ import annotations

from app.contracts.release_objects import EvidencePackage, ReleaseCandidate, RollbackTarget
from orchestration.evidence_package_validation import evidence_completeness_blocked_gates


def test_research_skips_completeness():
    ev = EvidencePackage(version_identifiers={"config": "1"})
    c = ReleaseCandidate(
        release_id="r",
        kind="config",
        owner="o",
        config_version="1",
        evidence=ev,
        rollback=RollbackTarget(target_config_version="0"),
    )
    blocked, _ = evidence_completeness_blocked_gates(c, ev, target_environment="research")
    assert blocked == []


def test_simulation_blocks_without_key_metrics():
    ev = EvidencePackage(
        version_identifiers={"config": "1.0.0"},
        replay_summary="ok",
        scenario_test_run_ids=["s1"],
        failure_modes_documented="x",
    )
    c = ReleaseCandidate(
        release_id="r",
        kind="config",
        owner="o",
        config_version="1.0.0",
        evidence=ev,
        rollback=RollbackTarget(target_config_version="0.9.0"),
    )
    blocked, _ = evidence_completeness_blocked_gates(c, ev, target_environment="simulation")
    assert "key_metrics" in blocked


def test_simulation_blocks_without_scenario_run_ids():
    ev = EvidencePackage(
        version_identifiers={"config": "1.0.0"},
        key_metrics={"k": 1.0},
        failure_modes_documented="x",
    )
    c = ReleaseCandidate(
        release_id="r",
        kind="config",
        owner="o",
        config_version="1.0.0",
        evidence=ev,
        rollback=RollbackTarget(target_config_version="0.9.0"),
    )
    blocked, _ = evidence_completeness_blocked_gates(c, ev, target_environment="simulation")
    assert "scenario_test_runs" in blocked


def test_shadow_requires_comparison_summary():
    ev = EvidencePackage(
        version_identifiers={"config": "1.0.0"},
        replay_summary="ok",
        scenario_test_run_ids=["s1"],
        key_metrics={"k": 1.0},
        failure_modes_documented="x",
        shadow_comparison_summary="",
    )
    c = ReleaseCandidate(
        release_id="r",
        kind="config",
        owner="o",
        config_version="1.0.0",
        evidence=ev,
        rollback=RollbackTarget(target_config_version="0.9.0"),
    )
    blocked, _ = evidence_completeness_blocked_gates(c, ev, target_environment="shadow")
    assert "shadow_comparison_summary" in blocked
