from __future__ import annotations

from enum import StrEnum

from pydantic import BaseModel, Field

from app.contracts.canonical_state import CanonicalStateOutput


class SemanticRegime(StrEnum):
    BULL = "bull"
    BEAR = "bear"
    VOLATILE = "volatile"
    SIDEWAYS = "sideways"


class RegimeOutput(BaseModel):
    """Regime view for the decision path.

    **Legacy:** ``probabilities`` holds the forecaster HMM soft vector (length 4: bull/bear/volatile/sideways).
    **Canonical (FB-CAN-041):** ``canonical_regime_probabilities`` holds the APEX 5-class vector
    (trend, range, stress, dislocated, transition) — use with ``apex`` for full state; do not rely on
    ``semantic`` / ``state_index`` alone for migration logic.
    """

    state_index: int = Field(ge=0, le=3)
    semantic: SemanticRegime
    probabilities: list[float]
    confidence: float = Field(ge=0.0, le=1.0)
    canonical_regime_probabilities: list[float] | None = Field(
        default=None,
        description="APEX 5-class regime distribution when canonical state is built (order: trend, range, stress, dislocated, transition)",
    )
    apex: CanonicalStateOutput | None = None
