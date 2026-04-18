"""FB-CAN-053 rollback playbook validation."""

from __future__ import annotations

from app.contracts.release_objects import RollbackTarget
from orchestration.rollback_validation import (
    validate_rollback_playbook,
    validate_rollback_target_references,
)


def test_playbook_required_beyond_research():
    rb = RollbackTarget(
        target_config_version="0.9.0",
        instructions="x" * 9,
        trigger_conditions="y" * 9,
        rollback_owner="ops",
    )
    ok, reasons = validate_rollback_playbook(rb, target_environment="live")
    assert ok is False
    assert any("instructions" in r for r in reasons)

    rb2 = RollbackTarget(
        target_config_version="0.9.0",
        instructions="revert config to prior version and restart",
        trigger_conditions="failed gates after deploy",
        rollback_owner="ops",
    )
    ok2, _ = validate_rollback_playbook(rb2, target_environment="live")
    assert ok2 is True


def test_research_skips_playbook():
    rb = RollbackTarget()
    ok, reasons = validate_rollback_playbook(rb, target_environment="research")
    assert ok is True
    assert reasons == []


def test_empty_version_string_rejected():
    rb = RollbackTarget(target_config_version="   ")
    ok, reasons = validate_rollback_target_references(rb)
    assert ok is False
    assert any("target_config_version" in r for r in reasons)
