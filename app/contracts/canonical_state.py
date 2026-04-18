"""APEX canonical state / regime outputs (FB-CAN-004).

Aligned with docs/Human Provided Specs/new_specs/canonical/APEX_State_Regime_Logic_Detail_Spec_v1_0.md
§4–11 (regime vector, confidence, transition, heat, reflexivity, degradation).
"""

from __future__ import annotations

from enum import StrEnum

from pydantic import BaseModel, Field


class DegradationLevel(StrEnum):
    NORMAL = "normal"
    REDUCED = "reduced"
    DEFENSIVE = "defensive"
    NO_TRADE = "no_trade"


class CanonicalStateOutput(BaseModel):
    """Compact, deterministic state snapshot for replay and downstream consumers."""

    regime_probabilities: list[float] = Field(
        ...,
        min_length=5,
        max_length=5,
        description="Order: trend, range, stress, dislocated, transition — sum ≈ 1",
    )
    regime_confidence: float = Field(ge=0.0, le=1.0)
    transition_probability: float = Field(ge=0.0, le=1.0)
    novelty: float = Field(ge=0.0, le=1.0)
    heat_score: float = Field(ge=0.0, le=1.0)
    reflexivity_score: float = Field(ge=0.0, le=1.0)
    degradation: DegradationLevel = DegradationLevel.NORMAL
    # Explainability (optional component breakdown for heat)
    heat_components: dict[str, float] = Field(default_factory=dict)
    # FB-CAN-042 — novelty / reflexivity trace (spec §8–10)
    novelty_components: dict[str, float] = Field(default_factory=dict)
    reflexivity_components: dict[str, float] = Field(default_factory=dict)
    novelty_reason_codes: list[str] = Field(default_factory=list)
    session_reason_codes: list[str] = Field(
        default_factory=list,
        description="FB-CAN-073: e.g. state_session_weekend, state_session_low_liquidity",
    )
    # FB-CAN-073 — weekend / low-liquidity session context (APEX State spec §12)
    session_mode: str = Field(
        default="regular",
        description="regular | weekend | low_liquidity — calendar + optional depth proxy",
    )
    session_mode_throttle: float = Field(
        default=1.0,
        ge=0.0,
        le=1.0,
        description="Aggression multiplier for sizing / trigger tightening (1.0 = no throttle)",
    )
    # FB-CAN-074 — venue / feed safety inputs (APEX State spec §13–14)
    exchange_risk_level: str = Field(
        default="low",
        description="low | elevated | high | critical — from boundary safety snapshot or feature hints",
    )
    data_integrity_alert: bool = Field(
        default=False,
        description="True when upstream marks data integrity warning / stale composite inputs",
    )
    safety_reason_codes: list[str] = Field(
        default_factory=list,
        description="state_* codes for exchange risk and data integrity (monitoring / replay)",
    )
