"""Canonical partial-fill reconciliation record (FB-CAN-048).

See APEX_Execution_Logic_Detail_Spec_v1_0.md §8.
"""

from __future__ import annotations

from pydantic import BaseModel, Field


class PartialFillReconciliation(BaseModel):
    """Deterministic outcome of one partial-fill reconciliation step (replay / gateway)."""

    outcome: str = Field(
        description="done | abandon | pause_or_reduce | continue_staggered",
    )
    intended_qty: float = Field(ge=0.0)
    filled_qty: float = Field(ge=0.0)
    residual_qty: float = Field(ge=0.0)
    fill_ratio: float = Field(ge=0.0, le=1.0)
    remaining_edge: float = 0.0
    execution_confidence_realized: float = Field(ge=0.0, le=1.0)
    cancel_replace_policy: str = Field(
        default="none",
        description="none | hold | reschedule_child — operator hint for residual handling",
    )
    reason_codes: list[str] = Field(default_factory=list)
