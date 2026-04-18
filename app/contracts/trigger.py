"""APEX canonical trigger output (FB-CAN-005, FB-CAN-043).

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
    # FB-CAN-043 — stage machine trace (ISO-8601, decision-time anchors; latency in ms)
    stage_timestamp_setup: str | None = None
    stage_timestamp_pretrigger: str | None = None
    stage_timestamp_confirm: str | None = None
    setup_to_confirm_latency_ms: float | None = Field(default=None, ge=0.0)
    stage_failure_codes: dict[str, list[str]] = Field(
        default_factory=dict,
        description="Per-stage failure reasons: keys setup | pretrigger | confirm",
    )
