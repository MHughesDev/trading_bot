"""FB-CAN-065: governance / operator Prometheus metrics."""

from __future__ import annotations

from app.contracts.release_objects import EvidencePackage, PromotionGateResult, ReleaseCandidate, RollbackTarget
from observability.governance_metrics import record_config_diff_report, record_promotion_gate_result


def test_record_promotion_gate_blocked_increments_counters():
    r = PromotionGateResult(
        allowed=False,
        target_environment="live",
        reasons=["x"],
        blocked_gates=["replay_evidence", "owner_approval"],
    )
    record_promotion_gate_result(r, kind="config")


def test_record_config_diff_report():
    record_config_diff_report(
        {
            "semantic_analysis": {
                "requires_operator_review": True,
                "breaking_change": False,
            }
        }
    )


def test_evaluate_promotion_gates_records_metrics():
    from orchestration.release_gating import evaluate_promotion_gates

    c = ReleaseCandidate(
        release_id="r-test",
        kind="config",
        owner="t",
        config_version="1.0.0",
        evidence=EvidencePackage(version_identifiers={"config": "1.0.0"}),
        rollback=RollbackTarget(target_config_version="0.9.0", instructions="x", rollback_owner="o"),
    )
    evaluate_promotion_gates(c, target_environment="research")
