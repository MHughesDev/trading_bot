"""PortfolioState — human policy spec §8.2."""

from __future__ import annotations

from dataclasses import dataclass
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
