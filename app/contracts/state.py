from __future__ import annotations

from dataclasses import dataclass, field
from datetime import UTC, datetime

from app.contracts.common import ExecutionMode, SemanticRegime, SystemMode


@dataclass(slots=True)
class SymbolState:
    symbol: str
    last_price: float | None = None
    last_bar_ts: datetime | None = None
    spread_bps: float | None = None
    position_qty: float = 0.0
    exposure_usd: float = 0.0
    regime: SemanticRegime | None = None
    updated_at: datetime = field(default_factory=lambda: datetime.now(UTC))


@dataclass(slots=True)
class PortfolioState:
    cash_usd: float = 100_000.0
    equity_usd: float = 100_000.0
    gross_exposure_usd: float = 0.0
    unrealized_pnl_usd: float = 0.0
    drawdown_pct: float = 0.0
    updated_at: datetime = field(default_factory=lambda: datetime.now(UTC))


@dataclass(slots=True)
class RuntimeState:
    system_mode: SystemMode = SystemMode.RUNNING
    execution_mode: ExecutionMode = ExecutionMode.PAPER
    portfolio: PortfolioState = field(default_factory=PortfolioState)
    symbols: dict[str, SymbolState] = field(default_factory=dict)
    updated_at: datetime = field(default_factory=lambda: datetime.now(UTC))
