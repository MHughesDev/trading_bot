"""Experiment record completeness (FB-CAN-054).

Aligned with ``APEX_Research_Experiment_Registry_Spec_v1_0.md`` §8–9: predeclared
metrics and explicit failure-mode capture before completion or promotion.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from models.registry.experiment_registry import ExperimentRecord

# Minimum non-whitespace length for failure-mode narrative (spec §9).
_MIN_FAILURE_MODES_LEN = 24

# Non-empty metric tokens (trimmed); reject placeholder single-char entries.
_MIN_METRIC_TOKEN_LEN = 2

# Outcome / decision text once the run has finished or is promoted (spec §3 `success_decision`).
_MIN_SUCCESS_DECISION_LEN = 8


def _nonempty_metrics(metrics: list[str]) -> list[str]:
    return [m.strip() for m in metrics if m and m.strip()]


def validate_predeclared_success_metrics(record: "ExperimentRecord") -> tuple[bool, list[str]]:
    """
    Require at least one substantive predeclared success metric (``metrics_defined_before_run``).

    Empty strings and single-character placeholders are ignored/rejected.
    """
    reasons: list[str] = []
    good = _nonempty_metrics(record.metrics_defined_before_run)
    good = [m for m in good if len(m) >= _MIN_METRIC_TOKEN_LEN]
    if not good:
        reasons.append(
            "metrics_defined_before_run must list at least one predeclared success metric "
            f"(each token at least {_MIN_METRIC_TOKEN_LEN} characters; spec §8)"
        )
    return len(reasons) == 0, reasons


def validate_success_decision_documented(record: "ExperimentRecord") -> tuple[bool, list[str]]:
    """Require a substantive success/outcome decision string (spec §3)."""
    reasons: list[str] = []
    blob = (record.success_decision or "").strip()
    if len(blob) < _MIN_SUCCESS_DECISION_LEN:
        reasons.append(
            "success_decision must summarize the outcome vs predeclared metrics "
            f"(minimum {_MIN_SUCCESS_DECISION_LEN} non-whitespace characters; spec §3)"
        )
    return len(reasons) == 0, reasons


def validate_failure_mode_documentation(record: "ExperimentRecord") -> tuple[bool, list[str]]:
    """Require explicit failure-mode narrative (spec §9)."""
    reasons: list[str] = []
    blob = (record.failure_modes_observed or "").strip()
    if len(blob) < _MIN_FAILURE_MODES_LEN:
        reasons.append(
            "failure_modes_observed must document observed or assessed failure modes "
            f"(minimum {_MIN_FAILURE_MODES_LEN} non-whitespace characters; spec §9)"
        )
    return len(reasons) == 0, reasons


def validate_experiment_promotion_readiness(record: "ExperimentRecord") -> tuple[bool, list[str]]:
    """All checks required before ``completed`` or any promotion-related status."""
    ok_m, rm = validate_predeclared_success_metrics(record)
    ok_s, rs = validate_success_decision_documented(record)
    ok_f, rf = validate_failure_mode_documentation(record)
    return ok_m and ok_s and ok_f, rm + rs + rf
