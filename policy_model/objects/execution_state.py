"""ExecutionState — human policy spec §8.3."""

from __future__ import annotations

from dataclasses import dataclass


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
