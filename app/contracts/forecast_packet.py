"""Probabilistic forecast packet — aligns with Human Provided Specs (policy + forecaster)."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any


@dataclass
class ForecastPacket:
    """
    Output of the forecaster / world model for downstream policy and diagnostics.

    Fields mirror `docs/Human Provided Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD` §8.1.
    Use list[float] for portability (Torch tensors can be converted at model boundaries).
    """

    timestamp: datetime
    horizons: list[int]
    q_low: list[float]
    q_med: list[float]
    q_high: list[float]
    interval_width: list[float]
    regime_vector: list[float]
    confidence_score: float | list[float]
    ensemble_variance: list[float]
    ood_score: float
    forecast_diagnostics: dict[str, Any] = field(default_factory=dict)
    # Master system pipeline spec (§8): versioned packet + training/serving lineage
    packet_schema_version: int = 1
    source_checkpoint_id: str | None = None

    def __post_init__(self) -> None:
        h = len(self.horizons)
        for name, seq in [
            ("q_low", self.q_low),
            ("q_med", self.q_med),
            ("q_high", self.q_high),
            ("interval_width", self.interval_width),
        ]:
            if len(seq) != h:
                raise ValueError(f"{name} length {len(seq)} != horizons {h}")
