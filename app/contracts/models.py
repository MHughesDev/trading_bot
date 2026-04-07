from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

from pydantic import BaseModel, ConfigDict, Field

from app.contracts.common import RouteId, SemanticRegime


class FeatureSnapshot(BaseModel):
    model_config = ConfigDict(extra="forbid")

    timestamp: datetime = Field(default_factory=lambda: datetime.now(UTC))
    symbol: str
    values: dict[str, float] = Field(default_factory=dict)


class RegimeOutput(BaseModel):
    model_config = ConfigDict(extra="forbid")

    symbol: str
    raw_state: int
    semantic_state: SemanticRegime
    probabilities: list[float]
    confidence: float = Field(ge=0.0, le=1.0)


class ForecastOutput(BaseModel):
    model_config = ConfigDict(extra="forbid")

    symbol: str
    horizon_returns: dict[int, float] = Field(default_factory=dict)
    volatility_estimate: float = 0.0
    confidence: float = Field(ge=0.0, le=1.0)
    uncertainty: float = Field(ge=0.0)
    metadata: dict[str, Any] = Field(default_factory=dict)


class RouteScore(BaseModel):
    model_config = ConfigDict(extra="forbid")

    route_id: RouteId
    score: float
    components: dict[str, float] = Field(default_factory=dict)
