from __future__ import annotations

from enum import StrEnum

from pydantic import BaseModel, Field

from app.contracts.canonical_state import DegradationLevel


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
