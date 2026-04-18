"""APEX canonical trigger output (FB-CAN-005).

See docs/Human Provided Specs/new_specs/canonical/APEX_Trigger_Math_Pseudocode_Detail_Spec_v1_0.md §9.
"""

from __future__ import annotations

from pydantic import BaseModel, Field


class TriggerOutput(BaseModel):
    """Three-stage trigger evaluation record (deterministic, replayable)."""

    setup_valid: bool
    setup_score: float = Field(ge=0.0, le=1.0)
    pretrigger_valid: bool
    pretrigger_score: float = Field(ge=0.0, le=1.0)
    trigger_valid: bool
    trigger_type: str = Field(
        description="imbalance_spike | volume_burst | structure_break | composite_confirmed | none"
    )
    trigger_strength: float = Field(ge=0.0, le=1.0)
    trigger_confidence: float = Field(ge=0.0, le=1.0)
    missed_move_flag: bool
    trigger_reason_codes: list[str] = Field(default_factory=list)
