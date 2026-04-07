from __future__ import annotations

from enum import StrEnum

from pydantic import BaseModel, Field


class SemanticRegime(StrEnum):
    BULL = "bull"
    BEAR = "bear"
    VOLATILE = "volatile"
    SIDEWAYS = "sideways"


class RegimeOutput(BaseModel):
    """Gaussian HMM (4 states) mapped to semantic regimes."""

    state_index: int = Field(ge=0, le=3)
    semantic: SemanticRegime
    probabilities: list[float]
    confidence: float = Field(ge=0.0, le=1.0)
