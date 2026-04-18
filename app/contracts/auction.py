"""APEX opportunity auction record (FB-CAN-006).

See APEX_Auction_Scoring_Constraints_Detail_Spec_v1_0.md §16.
"""

from __future__ import annotations

from typing import Any

from pydantic import BaseModel, Field


class AuctionCandidateRecord(BaseModel):
    """One scored candidate with explainable components."""

    symbol: str
    direction: int
    eligible: bool
    status: str = Field(description="selected | suppressed | rejected")
    auction_score: float
    components: dict[str, float] = Field(default_factory=dict)
    penalties: dict[str, float] = Field(default_factory=dict)
    reasons: list[str] = Field(default_factory=list)


class AuctionResult(BaseModel):
    """Outcome of a single-symbol auction step (replay-friendly)."""

    selected_symbol: str | None = None
    selected_direction: int | None = None
    selected_score: float | None = None
    records: list[AuctionCandidateRecord] = Field(default_factory=list)
    top_n_limit: int = 1
    max_notional_usd: float = 0.0
    selected_notional_usd: float = 0.0
    # FB-CAN-034 — deterministic thesis / cluster metadata for diversification penalties
    clustering_metadata: dict[str, Any] = Field(default_factory=dict)
