"""APEX Decision Service input snapshot contracts (FB-CAN-015).

Aligned with
``docs/Human Provided Specs/new_specs/canonical/APEX_Decision_Service_Feature_Schema_and_Data_Contracts_v1_0.md``
§5–9 (input families). Pydantic enforces ranges; builders in
``app.contracts.snapshot_builders`` project legacy feature rows into these models.
"""

from __future__ import annotations

from datetime import datetime
from enum import StrEnum
from typing import Any

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator


class SessionMode(StrEnum):
    REGULAR = "regular"
    WEEKEND = "weekend"
    LOW_LIQUIDITY = "low_liquidity"
    STRESSED = "stressed"


class ExchangeRiskLevel(StrEnum):
    LOW = "low"
    ELEVATED = "elevated"
    HIGH = "high"
    CRITICAL = "critical"


class OrderStyleUsed(StrEnum):
    LIMIT = "limit"
    MARKET = "market"
    TWAP = "twap"
    STAGGERED = "staggered"


class MarketSnapshot(BaseModel):
    """§5 — normalized market state for one decision cycle."""

    model_config = ConfigDict(extra="ignore")

    snapshot_id: str
    timestamp: datetime
    instrument_id: str
    venue_group: str = "kraken"
    last_price: float = Field(gt=0.0)
    mid_price: float = Field(gt=0.0)
    best_bid: float
    best_ask: float
    spread_bps: float = Field(ge=0.0)
    realized_vol_short: float = Field(ge=0.0)
    realized_vol_medium: float = Field(ge=0.0)
    book_imbalance: float
    depth_near_touch: float = Field(ge=0.0)
    trade_volume_short: float = Field(ge=0.0)
    volume_burst_score: float = Field(ge=0.0)
    market_freshness: float = Field(ge=0.0, le=1.0)
    market_reliability: float = Field(ge=0.0, le=1.0)
    session_mode: SessionMode = SessionMode.REGULAR
    microprice: float | None = None
    depth_bid_1pct: float | None = None
    depth_ask_1pct: float | None = None
    trade_count_short: int | None = None
    price_return_short: float | None = None
    price_return_medium: float | None = None
    local_structure_break_score: float | None = None
    exchange_health_score: float | None = None
    source_latency_ms: float | None = None

    @model_validator(mode="after")
    def _bid_ask_order(self) -> MarketSnapshot:
        if self.best_ask < self.best_bid:
            raise ValueError("best_ask must be >= best_bid")
        return self


class StructuralSignalSnapshot(BaseModel):
    """§6 — leverage-flow / structural features (placeholders allowed)."""

    model_config = ConfigDict(extra="ignore")

    snapshot_id: str
    timestamp: datetime
    instrument_id: str
    funding_rate: float = 0.0
    funding_rate_zscore: float = 0.0
    funding_velocity: float = 0.0
    open_interest: float = 0.0
    open_interest_delta_short: float = 0.0
    basis_bps: float = 0.0
    cross_exchange_divergence: float = 0.0
    liquidation_proximity_long: float = Field(ge=0.0, le=1.0, default=0.5)
    liquidation_proximity_short: float = Field(ge=0.0, le=1.0, default=0.5)
    liquidation_cluster_density_long: float = Field(ge=0.0, le=1.0, default=0.0)
    liquidation_cluster_density_short: float = Field(ge=0.0, le=1.0, default=0.0)
    liquidation_data_confidence: float = Field(ge=0.0, le=1.0, default=0.5)
    signal_freshness_structural: float = Field(ge=0.0, le=1.0)
    signal_reliability_structural: float = Field(ge=0.0, le=1.0)
    cascade_magnitude_estimate_long: float | None = None
    cascade_magnitude_estimate_short: float | None = None
    oi_concentration_score: float | None = None
    perp_spot_divergence_score: float | None = None
    gex_score: float | None = Field(default=None, ge=-1.0, le=1.0)
    iv_skew_score: float | None = Field(default=None, ge=-1.0, le=1.0)
    options_freshness: float | None = Field(default=None, ge=0.0, le=1.0)
    options_reliability: float | None = Field(default=None, ge=0.0, le=1.0)
    stablecoin_flow_proxy: float | None = Field(default=None, ge=-1.0, le=1.0)
    stablecoin_freshness: float | None = Field(default=None, ge=0.0, le=1.0)
    signal_source_count: int | None = None


class SafetyRegimeSnapshot(BaseModel):
    """§7 — safety / regime inputs (pre-decision; may be neutral before apex merge)."""

    model_config = ConfigDict(extra="ignore")

    snapshot_id: str
    timestamp: datetime
    instrument_id: str
    regime_probabilities: dict[str, float]
    regime_confidence: float = Field(ge=0.0, le=1.0)
    transition_probability: float = Field(ge=0.0, le=1.0)
    novelty_score: float = Field(ge=0.0, le=1.0)
    crypto_heat_score: float = Field(ge=0.0, le=1.0)
    reflexivity_score: float = Field(ge=0.0, le=1.0)
    degradation_level: str = Field(
        description="normal | reduced | defensive | no_trade",
    )
    weekend_mode: bool = False
    exchange_risk_level: ExchangeRiskLevel = ExchangeRiskLevel.LOW
    degradation_reason_codes: list[str] = Field(default_factory=list)
    volatility_circuit_breaker_active: bool | None = None
    data_integrity_alert: bool | None = None

    @field_validator("regime_probabilities")
    @classmethod
    def _prob_map(cls, v: dict[str, float]) -> dict[str, float]:
        for x in v.values():
            if x < 0 or x > 1:
                raise ValueError("regime probabilities must be in [0,1]")
        return v


class ExecutionFeedbackSnapshot(BaseModel):
    """§8 — realized execution quality (synthetic neutral when unknown)."""

    model_config = ConfigDict(extra="ignore")

    feedback_id: str
    timestamp: datetime
    instrument_id: str
    related_intent_id: str = "none"
    expected_fill_price: float
    realized_fill_price: float
    realized_slippage_bps: float
    fill_ratio: float = Field(ge=0.0, le=1.0)
    fill_latency_ms: float = Field(ge=0.0)
    execution_confidence_realized: float = Field(ge=0.0, le=1.0)
    venue_quality_score: float = Field(ge=0.0, le=1.0)
    partial_fill_flag: bool = False
    cancel_replace_count: int | None = None
    order_style_used: OrderStyleUsed | None = None
    execution_stress_flag: bool | None = None
    execution_anomaly_codes: list[str] = Field(default_factory=list)


class ServiceConfigurationSnapshot(BaseModel):
    """§ service config — versioned view for replay attribution."""

    model_config = ConfigDict(extra="ignore")

    snapshot_id: str
    timestamp: datetime
    config_version: str = "1.0.0"
    logic_version: str | None = None
    execution_mode: str = "paper"
    market_data_symbols: list[str] = Field(default_factory=list)
    bar_interval_seconds: int = 60
    extra: dict[str, Any] = Field(default_factory=dict)


class DecisionBoundaryInput(BaseModel):
    """Typed bundle at the decision boundary (spec §3 input families)."""

    model_config = ConfigDict(extra="ignore")

    market: MarketSnapshot
    structural: StructuralSignalSnapshot
    safety: SafetyRegimeSnapshot
    execution_feedback: ExecutionFeedbackSnapshot
    service_config: ServiceConfigurationSnapshot
