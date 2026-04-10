"""Core policy objects — see `docs/Human Provided Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD` §8."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass
class PortfolioState:
    equity: float
    cash: float
    position_units: float
    position_notional: float
    position_fraction: float
    entry_price: float | None
    unrealized_pnl: float
    realized_pnl: float
    current_leverage: float
    time_in_position: int
    last_action: dict[str, Any] | None
    last_trade_timestamp: Any | None


@dataclass
class ExecutionState:
    mid_price: float
    spread: float
    estimated_slippage: float
    estimated_fee_rate: float
    available_liquidity_score: float
    latency_proxy: float
    volatility_proxy: float
    order_book_imbalance: float | None = None
    recent_trade_flow: float | None = None


@dataclass
class PolicyRiskEnvelope:
    """Policy-facing risk limits (distinct from `app.contracts.risk.RiskState` system modes)."""

    max_abs_position_fraction: float
    max_position_delta_per_step: float
    max_leverage: float
    min_trade_notional: float
    cooldown_steps_remaining: int
    allow_long: bool
    allow_short: bool
    kill_switch_active: bool
    max_drawdown_limit: float
    concentration_limit: float
    volatility_limit: float
    daily_loss_limit_remaining: float


@dataclass
class PolicyObservation:
    forecast_features: list[float]
    portfolio_features: list[float]
    execution_features: list[float]
    risk_features: list[float]
    history_features: list[float] | None
    metadata: dict[str, Any] = field(default_factory=dict)


@dataclass
class PolicyAction:
    target_exposure: float
    action_diagnostics: dict[str, Any] = field(default_factory=dict)


@dataclass
class TargetPosition:
    target_fraction: float
    target_units: float | None
    target_notional: float | None
    target_leverage: float
    reason_codes: list[str]


@dataclass
class ApprovedTarget:
    approved: bool
    approved_target_fraction: float
    rejection_reasons: list[str]
    clamp_reasons: list[str]
    risk_diagnostics: dict[str, Any]


@dataclass
class ExecutionPlan:
    required_delta_fraction: float
    required_delta_notional: float
    execution_mode: str
    max_child_order_size: float | None
    urgency: float
    skip_execution: bool
    skip_reasons: list[str]
