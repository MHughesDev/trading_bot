"""FB-CAN-054: predeclared metrics + failure-mode documentation."""

from __future__ import annotations

import pytest

from models.registry.experiment_registry import (
    ExperimentRecord,
    ExperimentRegistry,
    upsert_experiment,
)
from models.registry.experiment_validation import (
    validate_experiment_promotion_readiness,
    validate_failure_mode_documentation,
    validate_predeclared_success_metrics,
    validate_success_decision_documented,
)
from orchestration.release_gating import ReleaseCandidate, validate_linked_experiments_for_promotion


def _complete_record(**overrides) -> ExperimentRecord:
    base = dict(
        experiment_id="exp-x",
        title="t",
        owner="o",
        hypothesis="h",
        metrics_defined_before_run=["false_positive_rate"],
        success_decision="Outcome meets the false_positive_rate target.",
        failure_modes_observed="Observed elevated no-trade rate under stale feed stress.",
        status="completed",
    )
    base.update(overrides)
    return ExperimentRecord(**base)


def test_metrics_rejects_placeholders():
    r = _complete_record(metrics_defined_before_run=["a", "  ", "ok_metric"])
    ok, reasons = validate_predeclared_success_metrics(r)
    assert ok is True
    assert reasons == []

    r2 = _complete_record(metrics_defined_before_run=["x"])
    ok2, reasons2 = validate_predeclared_success_metrics(r2)
    assert ok2 is False
    assert any("predeclared" in m.lower() for m in reasons2)


def test_failure_modes_min_length():
    r = _complete_record(failure_modes_observed="short")
    ok, reasons = validate_failure_mode_documentation(r)
    assert ok is False
    ok2, _ = validate_failure_mode_documentation(
        _complete_record(
            failure_modes_observed="This is long enough to document failure modes explicitly."
        )
    )
    assert ok2 is True


def test_success_decision_min_length():
    r = _complete_record(success_decision="tiny")
    ok, reasons = validate_success_decision_documented(r)
    assert ok is False
    ok2, _ = validate_success_decision_documented(
        _complete_record(success_decision="Accepted: metrics met pre-run thresholds.")
    )
    assert ok2 is True


def test_promotion_readiness_aggregate():
    ok, _ = validate_experiment_promotion_readiness(_complete_record())
    assert ok is True


def test_upsert_rejects_incomplete_completed():
    reg = ExperimentRegistry()
    bad = ExperimentRecord(
        experiment_id="exp-bad",
        title="t",
        owner="o",
        hypothesis="h",
        metrics_defined_before_run=["m1"],
        status="completed",
        failure_modes_observed="x" * 30,
    )
    with pytest.raises(ValueError, match="success_decision"):
        upsert_experiment(reg, bad)


def test_upsert_accepts_complete_completed():
    reg = ExperimentRegistry()
    good = _complete_record(experiment_id="exp-good")
    reg2 = upsert_experiment(reg, good)
    assert len(reg2.experiments) == 1


def test_validate_linked_skips_for_research():
    c = ReleaseCandidate(
        release_id="r1",
        kind="config",
        owner="o",
        config_version="1.0.0",
        linked_experiment_ids=["missing-id"],
    )
    ok, msgs = validate_linked_experiments_for_promotion(
        c, target_environment="research"
    )
    assert ok is True
    assert msgs == []
