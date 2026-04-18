"""FB-CAN-056: monitoring domain checklist passes."""

from __future__ import annotations

from observability.monitoring_domain_checklist import (
    MONITORING_DOMAIN_CHECKLIST,
    validate_monitoring_domain_coverage,
)


def test_checklist_covers_eleven_domains():
    assert len(MONITORING_DOMAIN_CHECKLIST) == 11
    assert set(MONITORING_DOMAIN_CHECKLIST) == {
        "system",
        "data",
        "state",
        "trigger",
        "auction",
        "risk",
        "execution",
        "carry",
        "drift",
        "replay_shadow",
        "governance",
    }


def test_validate_passes_in_test_process():
    ok, reasons = validate_monitoring_domain_coverage()
    assert ok, reasons
