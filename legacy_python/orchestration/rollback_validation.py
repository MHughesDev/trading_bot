"""Rollback target and playbook validation (FB-CAN-053).

APEX Config Management spec §10: rollback must be operationally explicit — owner,
instructions, and trigger conditions for promotions beyond research.
"""

from __future__ import annotations

from app.contracts.release_objects import ReleaseEnvironment, RollbackTarget

# Minimum lengths keep free-text fields from being placeholders in CI / ledger.
_MIN_INSTRUCTIONS_LEN = 10
_MIN_TRIGGER_LEN = 10


def validate_rollback_playbook(
    rb: RollbackTarget,
    *,
    target_environment: ReleaseEnvironment,
) -> tuple[bool, list[str]]:
    """
    Return (ok, reasons). For ``target_environment == research`` no playbook fields required.

    For simulation / shadow / live promotions, require non-empty ``rollback_owner``,
    ``instructions``, and ``trigger_conditions`` (spec §10).
    """
    if target_environment == "research":
        return True, []

    reasons: list[str] = []
    owner = (rb.rollback_owner or "").strip()
    if not owner:
        reasons.append("rollback_owner must be set for promotion beyond research (FB-CAN-053)")

    instr = (rb.instructions or "").strip()
    if len(instr) < _MIN_INSTRUCTIONS_LEN:
        reasons.append(
            f"rollback.instructions must be at least {_MIN_INSTRUCTIONS_LEN} characters "
            f"(operator playbook; got {len(instr)})"
        )

    trig = (rb.trigger_conditions or "").strip()
    if len(trig) < _MIN_TRIGGER_LEN:
        reasons.append(
            f"rollback.trigger_conditions must be at least {_MIN_TRIGGER_LEN} characters "
            f"(rollback trigger; got {len(trig)})"
        )

    return (len(reasons) == 0, reasons)


def validate_rollback_target_references(rb: RollbackTarget) -> tuple[bool, list[str]]:
    """
    Structural checks on version refs (non-empty strings when present).

    Returns (ok, reasons). Empty list means OK.
    """
    reasons: list[str] = []
    for field in ("target_config_version", "target_logic_version", "target_model_family_ref"):
        v = getattr(rb, field, None)
        if v is not None and isinstance(v, str) and not v.strip():
            reasons.append(f"rollback.{field} must be non-empty when set")

    return (len(reasons) == 0, reasons)


__all__ = [
    "validate_rollback_playbook",
    "validate_rollback_target_references",
]
