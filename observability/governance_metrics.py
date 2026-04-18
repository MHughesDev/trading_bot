"""Governance / operator health metrics (FB-CAN-065).

Counters for promotion attempts, gate outcomes, per-gate failures, config diff / drift
signals, and rollback events. Import this module so instruments register with Prometheus
(see ``observability/monitoring_domain_checklist.py``).
"""

from __future__ import annotations

from typing import Any

from prometheus_client import Counter

GOVERNANCE_PROMOTION_ATTEMPT = Counter(
    "tb_governance_promotion_attempt",
    "Release promotion gate evaluations (API or tooling)",
    ["target_environment", "kind"],
)
GOVERNANCE_GATE_OUTCOME = Counter(
    "tb_governance_gate_outcome",
    "Gate evaluation outcomes",
    ["target_environment", "outcome"],
)
GOVERNANCE_GATE_FAILURE = Counter(
    "tb_governance_gate_failure",
    "Blocked gate id per evaluation (one increment per failing gate)",
    ["target_environment", "gate"],
)
GOVERNANCE_CONFIG_DRIFT_EVENT = Counter(
    "tb_governance_config_drift_event",
    "Canonical config diff evaluations (control plane / operator)",
    ["requires_operator_review", "breaking_change"],
)
GOVERNANCE_ROLLBACK_EVENT = Counter(
    "tb_governance_rollback_event",
    "Rollback-related release ledger events",
    ["source"],
)


def record_promotion_gate_result(
    result: Any,
    *,
    kind: str,
) -> None:
    """Record metrics from :class:`app.contracts.release_objects.PromotionGateResult`."""
    te = str(getattr(result, "target_environment", "unknown") or "unknown")
    k = (kind or "unknown").strip() or "unknown"
    GOVERNANCE_PROMOTION_ATTEMPT.labels(target_environment=te, kind=k).inc()
    allowed = bool(getattr(result, "allowed", False))
    outcome = "allowed" if allowed else "blocked"
    GOVERNANCE_GATE_OUTCOME.labels(target_environment=te, outcome=outcome).inc()
    if not allowed:
        for g in getattr(result, "blocked_gates", None) or []:
            gate = str(g or "unknown").strip() or "unknown"
            GOVERNANCE_GATE_FAILURE.labels(target_environment=te, gate=gate).inc()


def record_config_diff_report(report: dict[str, Any]) -> None:
    """Record one config diff evaluation (semantic flags from FB-CAN-057 reports)."""
    sem = report.get("semantic_analysis") if isinstance(report, dict) else None
    if not isinstance(sem, dict):
        sem = {}
    rev = bool(sem.get("requires_operator_review"))
    brk = bool(sem.get("breaking_change"))
    GOVERNANCE_CONFIG_DRIFT_EVENT.labels(
        requires_operator_review=str(rev).lower(),
        breaking_change=str(brk).lower(),
    ).inc()


def record_rollback_release_candidate(*, source: str = "release_object_write") -> None:
    """Increment when a release candidate is written with rollback lifecycle stage."""
    GOVERNANCE_ROLLBACK_EVENT.labels(source=str(source or "unknown")).inc()


__all__ = [
    "GOVERNANCE_CONFIG_DRIFT_EVENT",
    "GOVERNANCE_GATE_FAILURE",
    "GOVERNANCE_GATE_OUTCOME",
    "GOVERNANCE_PROMOTION_ATTEMPT",
    "GOVERNANCE_ROLLBACK_EVENT",
    "record_config_diff_report",
    "record_promotion_gate_result",
    "record_rollback_release_candidate",
]
