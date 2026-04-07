"""Risk engine: hard constraints + system modes; final authority before execution."""

from __future__ import annotations

from datetime import UTC, datetime
from decimal import Decimal

from app.config.settings import AppSettings
from app.contracts.decisions import ActionProposal, RouteId, TradeAction
from app.contracts.orders import OrderIntent, OrderSide, OrderType, TimeInForce
from app.contracts.risk import RiskState, SystemMode


class RiskEngine:
    def __init__(self, settings: AppSettings) -> None:
        self._settings = settings
        self._peak_equity = 100_000.0
        self._current_equity = 100_000.0

    def update_equity(self, equity: float) -> None:
        self._current_equity = equity
        self._peak_equity = max(self._peak_equity, equity)

    def evaluate(
        self,
        symbol: str,
        proposal: ActionProposal | None,
        risk: RiskState,
        *,
        mid_price: float,
        spread_bps: float,
        data_timestamp: datetime | None,
        current_total_exposure_usd: float = 0.0,
    ) -> tuple[TradeAction | None, RiskState]:
        """Return None if blocked; otherwise TradeAction for execution layer."""
        now = datetime.now(UTC)
        if data_timestamp is not None:
            dt = data_timestamp if data_timestamp.tzinfo else data_timestamp.replace(tzinfo=UTC)
            dt = dt.astimezone(UTC)
            age = abs((now - dt).total_seconds())
            risk = risk.model_copy(update={"data_age_seconds": age, "spread_bps": spread_bps})
            if age > self._settings.risk_stale_data_seconds:
                return None, risk

        if spread_bps > self._settings.risk_max_spread_bps:
            return None, risk.model_copy(update={"spread_bps": spread_bps})

        dd = 0.0
        if self._peak_equity > 0:
            dd = (self._peak_equity - self._current_equity) / self._peak_equity
        risk = risk.model_copy(update={"current_drawdown_pct": dd})
        if dd > self._settings.risk_max_drawdown_pct:
            return None, risk

        if proposal is None:
            return None, risk

        mode = risk.mode
        if mode == SystemMode.MAINTENANCE or mode == SystemMode.FLATTEN_ALL:
            return None, risk
        if mode == SystemMode.PAUSE_NEW_ENTRIES:
            return None, risk

        notional = proposal.size_fraction * self._settings.risk_max_per_symbol_usd
        if notional > self._settings.risk_max_per_symbol_usd:
            return None, risk
        if current_total_exposure_usd + notional > self._settings.risk_max_total_exposure_usd:
            return None, risk

        qty = Decimal(str(round(notional / max(mid_price, 1e-12), 8)))
        if qty <= 0:
            return None, risk

        side = OrderSide.BUY if proposal.direction > 0 else OrderSide.SELL
        if mode == SystemMode.REDUCE_ONLY and proposal.route_id != RouteId.NO_TRADE:
            # reduce-only: only allow closes — V1 stub: block new risk-increasing trades
            return None, risk

        action = TradeAction(
            symbol=symbol,
            side=side.value,
            quantity=qty,
            order_type=proposal.order_type,
            limit_price=None,
            stop_price=None,
            time_in_force="gtc",
            route_id=proposal.route_id,
        )
        return action, risk

    def to_order_intent(self, action: TradeAction) -> OrderIntent:
        return OrderIntent(
            symbol=action.symbol,
            side=OrderSide(action.side),
            quantity=action.quantity,
            order_type=OrderType.MARKET if action.order_type == "market" else OrderType.LIMIT,
            limit_price=action.limit_price,
            stop_price=action.stop_price,
            time_in_force=TimeInForce.GTC,
            metadata={"route_id": action.route_id.value},
        )
