"""PolicyObservation — human policy spec §8.5."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass
class PolicyObservation:
    forecast_features: list[float]
    portfolio_features: list[float]
    execution_features: list[float]
    risk_features: list[float]
    history_features: list[float] | None
    metadata: dict[str, Any] = field(default_factory=dict)
