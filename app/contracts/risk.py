from __future__ import annotations

from enum import StrEnum

from typing import Any

from pydantic import BaseModel, Field

from app.contracts.canonical_state import DegradationLevel
from app.contracts.hard_override import HardOverrideKind


class SystemMode(StrEnum):
    RUNNING = "RUNNING"
    PAUSE_NEW_ENTRIES = "PAUSE_NEW_ENTRIES"
    REDUCE_ONLY = "REDUCE_ONLY"
    FLATTEN_ALL = "FLATTEN_ALL"
    MAINTENANCE = "MAINTENANCE"


class RiskState(BaseModel):
    mode: SystemMode = SystemMode.RUNNING
    current_drawdown_pct: float = 0.0
    spread_bps: float | None = None
    data_age_seconds: float | None = None
    # APEX canonical degradation (FB-CAN-004); size_multiplier throttles new risk
    canonical_degradation: DegradationLevel | None = None
    canonical_size_multiplier: float = Field(default=1.0, ge=0.0, le=1.0)
    # FB-CAN-007 — inputs for canonical layered sizing (set on decision hot path)
    risk_asymmetry_score: float | None = Field(default=None, ge=0.0, le=1.0)
    risk_trigger_confidence: float | None = Field(default=None, ge=0.0, le=1.0)
    risk_execution_confidence: float | None = Field(default=None, ge=0.0, le=1.0)
    risk_heat_score: float | None = Field(default=None, ge=0.0, le=1.0)
    risk_novelty_score: float | None = Field(default=None, ge=0.0, le=1.0)
    risk_reflexivity_score: float | None = Field(default=None, ge=0.0, le=1.0)
    risk_liquidation_mode: str | None = None
    last_risk_sizing: dict | None = None
    # FB-CAN-016 — canonical feature normalization at bar/tick boundary
    feature_freshness: float | None = Field(default=None, ge=0.0, le=1.0)
    feature_reliability: float | None = Field(default=None, ge=0.0, le=1.0)
    signal_confidence_aggregate: float | None = Field(default=None, ge=0.0, le=1.0)
    canonical_snapshot_complete: float | None = Field(default=None, ge=0.0, le=1.0)
    # FB-CAN-018 — carry sleeve snapshot (last tick; replay / audit)
    carry_sleeve_last: dict | None = None
    # FB-CAN-031 — false-positive / late-chase memory [0,1] for auction penalty (deterministic EMA)
    trigger_false_positive_memory: float = Field(default=0.0, ge=0.0, le=1.0)
    # FB-CAN-033 — hard override + degradation transition accounting (deterministic, replay-friendly)
    hard_override_active: bool = False
    hard_override_kind: HardOverrideKind = HardOverrideKind.NONE
    degradation_transition_count: int = Field(default=0, ge=0)
    last_degradation_level: str | None = None
    degradation_occupancy_ticks: dict[str, int] = Field(default_factory=dict)
    # FB-CAN-036 — last-tick canonical reason codes (replay / decision record)
    last_risk_block_codes: list[str] = Field(default_factory=list)
    last_pipeline_no_trade_codes: list[str] = Field(default_factory=list)
    last_decision_record: dict[str, Any] | None = None
