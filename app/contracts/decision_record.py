"""Canonical decision outputs and audit record (FB-CAN-036).

Aligned with APEX_Decision_Service_Feature_Schema_and_Data_Contracts_v1_0.md §13–15.
"""

from __future__ import annotations

from datetime import datetime
from enum import StrEnum
from typing import Any

from pydantic import BaseModel, Field

from app.contracts.run_binding import RunBinding


class TradeIntentSide(StrEnum):
    LONG = "long"
    SHORT = "short"
    REDUCE = "reduce"
    FLAT = "flat"


class Urgency(StrEnum):
    LOW = "low"
    MEDIUM = "medium"
    HIGH = "high"


class PreferredExecutionStyle(StrEnum):
    PASSIVE = "passive"
    AGGRESSIVE = "aggressive"
    STAGGERED = "staggered"
    TWAP = "twap"
    SUPPRESS = "suppress"


class SuppressionType(StrEnum):
    AUCTION = "auction"
    CARRY_ISOLATION = "carry_isolation"
    EXECUTION_GUIDANCE = "execution_guidance"
    RISK_ENGINE = "risk_engine"
    OTHER = "other"


class SafetyOverrideType(StrEnum):
    HARD_OVERRIDE = "hard_override"
    SYSTEM_MODE = "system_mode"
    NONE = "none"


class DecisionOutcome(StrEnum):
    TRADE_INTENT = "trade_intent"
    REDUCE_EXPOSURE = "reduce_exposure"
    NO_TRADE = "no_trade"
    SUPPRESSED = "suppressed"


class TradeIntentCanonical(BaseModel):
    """§13 — emitted when a directional trade is selected (pre-risk or post-policy)."""

    intent_id: str
    timestamp: datetime
    instrument_id: str
    side: TradeIntentSide
    urgency: Urgency = Urgency.MEDIUM
    size_fraction: float = Field(ge=0.0, le=1.0)
    preferred_execution_style: PreferredExecutionStyle = PreferredExecutionStyle.STAGGERED
    decision_confidence: float = Field(ge=0.0, le=1.0, default=0.0)
    trigger_confidence: float = Field(ge=0.0, le=1.0, default=0.0)
    execution_confidence: float = Field(ge=0.0, le=1.0, default=0.0)
    degradation_level: str = "normal"
    max_slippage_tolerance_bps: float = Field(ge=0.0, default=0.0)
    reason_codes: list[str] = Field(default_factory=list)


class ReduceExposureIntent(BaseModel):
    """Flatten / reduce path (e.g. FLATTEN_ALL)."""

    intent_id: str
    timestamp: datetime
    instrument_id: str
    side: TradeIntentSide = TradeIntentSide.REDUCE
    urgency: Urgency = Urgency.HIGH
    size_fraction: float = Field(ge=0.0, le=1.0, default=1.0)
    reason_codes: list[str] = Field(default_factory=list)


class NoTradeDecision(BaseModel):
    """§14.2 — completed cycle with no trade."""

    event_id: str
    timestamp: datetime
    instrument_id: str
    no_trade_reason_codes: list[str] = Field(default_factory=list)
    state_summary: dict[str, Any] = Field(default_factory=dict)


class SuppressionEvent(BaseModel):
    """§14.1 — valid candidate blocked intentionally."""

    event_id: str
    timestamp: datetime
    instrument_id: str
    suppression_type: SuppressionType
    reason_codes: list[str] = Field(default_factory=list)
    blocked_candidate_id: str = ""
    degradation_level: str = "normal"


class SafetyOverrideEvent(BaseModel):
    """§14.3 — hard safety override."""

    event_id: str
    timestamp: datetime
    override_type: SafetyOverrideType
    reason_codes: list[str] = Field(default_factory=list)
    affected_instruments: list[str] = Field(default_factory=list)


class DecisionRecord(BaseModel):
    """§15 — full replayable audit object for one decision cycle."""

    schema_version: int = 1
    record_id: str
    timestamp: datetime
    instrument_id: str
    config_version: str = "1.0.0"
    logic_version: str | None = None
    run_binding: RunBinding | None = None
    input_snapshot_ids: dict[str, str] = Field(default_factory=dict)
    effective_signal_map: dict[str, float] = Field(default_factory=dict)
    regime_semantic: str | None = None
    degradation_level: str | None = None
    forecast_summary: dict[str, Any] = Field(default_factory=dict)
    trigger_output: dict[str, Any] | None = None
    auction_summary: dict[str, Any] | None = None
    selected_route: str | None = None
    outcome: DecisionOutcome = DecisionOutcome.NO_TRADE
    trade_intent: TradeIntentCanonical | None = None
    reduce_exposure_intent: ReduceExposureIntent | None = None
    no_trade: NoTradeDecision | None = None
    suppression: SuppressionEvent | None = None
    safety_override: SafetyOverrideEvent | None = None
    risk_block_codes: list[str] = Field(default_factory=list)
    pipeline_no_trade_codes: list[str] = Field(default_factory=list)
    diagnostics: dict[str, Any] = Field(default_factory=dict)


__all__ = [
    "DecisionOutcome",
    "DecisionRecord",
    "NoTradeDecision",
    "PreferredExecutionStyle",
    "ReduceExposureIntent",
    "SafetyOverrideEvent",
    "SafetyOverrideType",
    "SuppressionEvent",
    "SuppressionType",
    "TradeIntentCanonical",
    "TradeIntentSide",
    "Urgency",
]
