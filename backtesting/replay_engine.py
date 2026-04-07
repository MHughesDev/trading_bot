from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any

from app.contracts.decisions import ExecutionReport, OrderIntent
from app.contracts.events import BarEvent
from app.contracts.state import PortfolioState


@dataclass(slots=True)
class ReplayResult:
    equity_curve: list[tuple[datetime, float]] = field(default_factory=list)
    orders: list[OrderIntent] = field(default_factory=list)
    fills: list[ExecutionReport] = field(default_factory=list)
    metrics: dict[str, float] = field(default_factory=dict)


@dataclass(slots=True)
class ExecutionSimulator:
    slippage_bps: float = 2.0

    def simulate_fill(self, order: OrderIntent, bar: BarEvent) -> ExecutionReport:
        base_price = bar.close
        price = base_price * (1 + self.slippage_bps / 10_000)
        return ExecutionReport(
            order_id=f"sim_{order.decision_id}",
            client_order_id=order.decision_id,
            symbol=order.symbol,
            side=order.side,
            quantity=order.quantity,
            filled_quantity=order.quantity,
            avg_fill_price=price,
            status="filled",
            adapter="backtest_simulator",
            raw={"slippage_bps": self.slippage_bps},
        )


@dataclass(slots=True)
class PortfolioTracker:
    state: PortfolioState = field(default_factory=PortfolioState)

    def mark_to_market(self, timestamp: datetime) -> tuple[datetime, float]:
        return (timestamp, self.state.equity_usd)

    def apply_fill(self, fill: ExecutionReport) -> None:
        if fill.avg_fill_price is None:
            return
        notional = fill.avg_fill_price * fill.filled_quantity
        self.state.gross_exposure_usd += abs(notional)
        self.state.cash_usd -= notional
        self.state.equity_usd = self.state.cash_usd + self.state.gross_exposure_usd


@dataclass(slots=True)
class ReplayEngine:
    simulator: ExecutionSimulator = field(default_factory=ExecutionSimulator)

    def run(
        self,
        bars: Iterable[BarEvent],
        decision_callback: Any,
    ) -> ReplayResult:
        """
        Replay bars through a shared decision callback.

        decision_callback signature:
            (bar: BarEvent) -> OrderIntent | None
        """
        tracker = PortfolioTracker()
        result = ReplayResult()
        bars_list = list(bars)

        for bar in bars_list:
            order = decision_callback(bar)
            if order is not None:
                fill = self.simulator.simulate_fill(order, bar)
                tracker.apply_fill(fill)
                result.orders.append(order)
                result.fills.append(fill)
            result.equity_curve.append(tracker.mark_to_market(bar.timestamp))

        if result.equity_curve:
            start_eq = result.equity_curve[0][1]
            end_eq = result.equity_curve[-1][1]
            result.metrics["pnl"] = end_eq - start_eq
            result.metrics["return_pct"] = ((end_eq / start_eq) - 1.0) if start_eq else 0.0
            result.metrics["num_orders"] = float(len(result.orders))
        return result
