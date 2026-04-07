from __future__ import annotations

from pydantic import BaseModel, Field


class ForecastOutput(BaseModel):
    """TFT-style forecast (steps as bar horizons, not necessarily minutes)."""

    returns_1: float
    returns_3: float
    returns_5: float
    returns_15: float
    volatility: float = Field(ge=0.0)
    uncertainty: float = Field(ge=0.0, description="Epistemic / confidence inverse")
