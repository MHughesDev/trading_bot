"""APEX execution guidance output (FB-CAN-008).

See APEX_Execution_Logic_Detail_Spec_v1_0.md §12.
"""

from __future__ import annotations

from pydantic import BaseModel, Field


class ExecutionGuidance(BaseModel):
    """Deterministic, replay-friendly execution plan attached to OrderIntent metadata."""

    preferred_execution_style: str = Field(
        description="passive | aggressive | staggered | twap | suppress"
    )
    execution_confidence: float = Field(ge=0.0, le=1.0)
    max_slippage_tolerance_bps: float = Field(ge=0.0)
    stress_mode_flag: bool = False
    venue_preference_order: list[str] = Field(default_factory=list)
    execution_reason_codes: list[str] = Field(default_factory=list)
    # FB-CAN-047 — deterministic style branch codes (subset of execution_reason_codes for style path)
    style_rationale_codes: list[str] = Field(default_factory=list)
    worst_case_edge: float = 0.0
    remaining_edge: float = Field(
        default=0.0,
        description="Pre-trade remaining edge proxy used for urgency / aggressive branch",
    )
    urgency_high: bool = False
    suppress_order: bool = False
    size_multiplier: float = Field(default=1.0, ge=0.0, le=1.0)
    # FB-CAN-075 — explainable execution confidence decomposition (APEX Execution spec §4)
    execution_confidence_terms: dict[str, float] = Field(
        default_factory=dict,
        description="Per-term quality [0,1] and weights used in execution_confidence",
    )


class ExecutionFeedback(BaseModel):
    """Post-trade feedback for reconciliation and trust updates."""

    fill_ratio: float = Field(ge=0.0, le=1.0, default=1.0)
    realized_slippage_bps: float = 0.0
    fill_latency_ms: float | None = None
    venue_quality_score: float = Field(ge=0.0, le=1.0, default=0.8)
    partial_fill_flag: bool = False
    adapter: str = ""
