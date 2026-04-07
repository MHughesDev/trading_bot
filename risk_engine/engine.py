from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime

from app.config.settings import RiskSettings
from app.contracts.common import Side, SystemMode
from app.contracts.decisions import OrderIntent, RiskDecision
from app.contracts.state import RuntimeState


@dataclass(slots=True)
class RiskEngine:
    settings: RiskSettings

    def evaluate(
        self,
        order: OrderIntent,
        runtime_state: RuntimeState,
        spread_bps: float | None,
        last_market_ts: datetime | None,
        mark_price: float,
    ) -> RiskDecision:
        blocked_by: list[str] = []

        system_mode = runtime_state.system_mode
        if system_mode in {SystemMode.MAINTENANCE, SystemMode.FLATTEN_ALL}:
            blocked_by.append(f"system_mode={system_mode.value}")

        if system_mode == SystemMode.PAUSE_NEW_ENTRIES:
            symbol_pos = runtime_state.symbols.get(order.symbol)
            position_qty = symbol_pos.position_qty if symbol_pos else 0.0
            # Allow only position-reducing orders.
            if not self._is_reducing_order(order.quantity, position_qty, order.side):
                blocked_by.append("pause_new_entries_block")

        if spread_bps is not None and spread_bps > self.settings.max_spread_bps:
            blocked_by.append("spread_threshold_exceeded")

        if last_market_ts is None:
            blocked_by.append("stale_data_no_timestamp")
        else:
            age_s = (datetime.now(UTC) - last_market_ts).total_seconds()
            if age_s > self.settings.stale_data_seconds:
                blocked_by.append("stale_data_guard")

        notional = order.quantity * max(mark_price, 0.0)
        if notional > self.settings.max_order_notional_usd:
            scale = self.settings.max_order_notional_usd / max(notional, 1e-8)
            adjusted_qty = max(order.quantity * scale, 0.0)
        else:
            adjusted_qty = order.quantity

        portfolio = runtime_state.portfolio
        if portfolio.gross_exposure_usd > self.settings.max_total_exposure_usd:
            blocked_by.append("max_total_exposure")

        symbol_state = runtime_state.symbols.get(order.symbol)
        symbol_exposure = symbol_state.exposure_usd if symbol_state else 0.0
        if symbol_exposure > self.settings.max_symbol_exposure_usd:
            blocked_by.append("max_symbol_exposure")

        if portfolio.drawdown_pct > self.settings.max_drawdown_pct:
            blocked_by.append("max_drawdown")

        if blocked_by:
            return RiskDecision(
                approved=False,
                reason="risk_rule_blocked",
                adjusted_quantity=None,
                blocked_by=blocked_by,
            )

        if adjusted_qty <= 0:
            return RiskDecision(
                approved=False,
                reason="adjusted_quantity_non_positive",
                adjusted_quantity=0.0,
                blocked_by=["max_order_notional_usd"],
            )

        if adjusted_qty < order.quantity:
            return RiskDecision(
                approved=True,
                reason="approved_with_size_adjustment",
                adjusted_quantity=adjusted_qty,
                blocked_by=[],
            )

        return RiskDecision(
            approved=True,
            reason="approved",
            adjusted_quantity=order.quantity,
            blocked_by=[],
        )

    def _is_reducing_order(self, order_qty: float, position_qty: float, side: Side) -> bool:
        if position_qty == 0:
            return False
        if position_qty > 0 and side == Side.SELL:
            return True
        if position_qty < 0 and side == Side.BUY:
            return True
        # If same-side as current position, it expands risk.
        return False
