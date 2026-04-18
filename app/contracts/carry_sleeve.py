"""APEX carry sleeve decision contract (FB-CAN-018).

See APEX_Canonical_Configuration_Spec_v1_0.md §14 and master spec carry domain.
"""

from __future__ import annotations

from pydantic import BaseModel, Field


class CarrySleeveDecision(BaseModel):
    """Deterministic carry evaluation for one symbol tick (replay-auditable)."""

    eligible: bool = False
    active: bool = False
    reason_codes: list[str] = Field(default_factory=list)
    funding_signal: float = 0.0
    trigger_confidence: float = Field(
        default=0.0,
        ge=0.0,
        le=1.0,
        description="Trigger confidence at evaluation time (monitoring / replay, FB-CAN-064).",
    )
    decision_quality: float = Field(
        default=0.0,
        ge=0.0,
        le=1.0,
        description="funding × trigger confidence in [0,1] when carry is evaluated (FB-CAN-064).",
    )
    target_notional_usd: float = Field(default=0.0, ge=0.0)
    directional_blocked: bool = False
    isolation_required: bool = True
