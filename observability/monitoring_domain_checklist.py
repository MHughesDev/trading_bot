"""Machine-checkable APEX monitoring domain coverage (FB-CAN-056).

Maps the eleven domains in ``APEX_Monitoring_and_Alerting_Spec_v1_0.md`` §3–4 to
required Prometheus metric **names** that must remain registered when the canonical
observability modules are loaded.

CI: ``scripts/ci_monitoring_domain_checklist.py`` (also run from ``ci_canonical_gates.sh``).
"""

from __future__ import annotations

from typing import FrozenSet

# Required metric **family** names per monitoring domain (spec §3 — system … governance).
# Use names as returned by ``REGISTRY.collect()`` (``MetricFamily.name``). For Counters,
# the Python client often registers the name **without** the ``_total`` suffix that
# appears in Prometheus exposition text.
MONITORING_DOMAIN_CHECKLIST: dict[str, frozenset[str]] = {
    "system": frozenset(
        {
            "tb_decision_latency_seconds",
        }
    ),
    "data": frozenset(
        {
            "tb_canonical_data_age_seconds",
            "tb_canonical_options_context_available",
            "tb_canonical_stablecoin_flow_available",
        }
    ),
    "state": frozenset(
        {
            "tb_canonical_regime_confidence",
            "tb_canonical_degradation_observations",
            "tb_canonical_hard_override",
        }
    ),
    "trigger": frozenset(
        {
            "tb_canonical_trigger_stage",
            "tb_canonical_trigger_missed_move",
        }
    ),
    "auction": frozenset(
        {
            "tb_canonical_auction_suppressed",
            "tb_canonical_auction_candidates_evaluated",
        }
    ),
    "risk": frozenset(
        {
            "tb_canonical_risk_size_multiplier",
            "tb_canonical_risk_final_notional_usd",
        }
    ),
    "execution": frozenset(
        {
            "tb_canonical_execution_style",
            "tb_canonical_trade_intent_execution_confidence",
        }
    ),
    "carry": frozenset(
        {
            "tb_canonical_carry_sleeve_active",
            "tb_canonical_carry_target_notional_usd",
            "tb_canonical_carry_funding_signal",
            "tb_canonical_carry_trigger_confidence",
            "tb_canonical_carry_decision_quality",
            "tb_canonical_carry_reason",
            "tb_canonical_carry_directional_suppression",
        }
    ),
    "drift": frozenset(
        {
            "tb_canonical_forecast_ood_score",
            "tb_canonical_feature_drift_penalty",
        }
    ),
    "replay_shadow": frozenset(
        {
            "tb_canonical_shadow_divergence_rate",
            "tb_canonical_replay_shadow_divergence",
        }
    ),
    "governance": frozenset(
        {
            "tb_canonical_active_config_version_info",
            "tb_governance_promotion_attempt",
            "tb_governance_gate_outcome",
            "tb_governance_gate_failure",
            "tb_governance_config_drift_event",
            "tb_governance_rollback_event",
        }
    ),
}


def _registered_metric_names() -> FrozenSet[str]:
    """Collect metric names from the Prometheus registry after loading canonical modules."""
    # Import side-effect: register instruments
    import observability.canonical_metrics  # noqa: F401
    import observability.drift_calibration_metrics  # noqa: F401
    import observability.governance_metrics  # noqa: F401
    import observability.metrics  # noqa: F401

    from prometheus_client import REGISTRY

    names: set[str] = set()
    for mf in REGISTRY.collect():
        names.add(mf.name)
    return frozenset(names)


def validate_monitoring_domain_coverage() -> tuple[bool, list[str]]:
    """
    Return (ok, error messages). Fails if any required metric name is not registered.
    """
    registered = _registered_metric_names()
    reasons: list[str] = []
    for domain, required in sorted(MONITORING_DOMAIN_CHECKLIST.items()):
        missing = sorted(required - registered)
        if missing:
            reasons.append(f"domain {domain!r} missing metrics: {missing!r}")
    return len(reasons) == 0, reasons


__all__ = [
    "MONITORING_DOMAIN_CHECKLIST",
    "validate_monitoring_domain_coverage",
]
