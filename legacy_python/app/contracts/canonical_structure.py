"""APEX canonical Forecast / Structure domain output (FB-CAN-017).

Aligned with APEX Decision Service data contracts §10 (forecast/structure) and
master spec §8 — first-class structure view for trigger and auction (not only raw quantiles).
"""

from __future__ import annotations

from pydantic import BaseModel, Field


class CanonicalStructureOutput(BaseModel):
    """Canonical structure + forecast tail summary (replay-friendly)."""

    p05: float
    p25: float
    p50: float
    p75: float
    p95: float
    volatility_forecast: float = Field(ge=0.0)
    asymmetry_score: float = Field(ge=0.0, le=1.0)
    continuation_probability: float = Field(ge=0.0, le=1.0)
    fragility_score: float = Field(ge=0.0, le=1.0)
    directional_bias: float = Field(ge=-1.0, le=1.0)
    model_agreement_score: float = Field(ge=0.0, le=1.0)
    model_correlation_penalty: float = Field(ge=0.0, le=1.0)
    calibration_weight: float = Field(ge=0.0, le=1.0, default=1.0)
    oi_structure_class: str = "unknown"
