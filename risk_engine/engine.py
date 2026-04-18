"""Risk engine: hard constraints + system modes; final authority before execution."""

from __future__ import annotations

from datetime import UTC, datetime
from decimal import Decimal

from app.config.settings import AppSettings
from app.contracts.decisions import ActionProposal, RouteId, TradeAction
from app.contracts.orders import OrderIntent, OrderSide, OrderType, TimeInForce
from app.contracts.risk import RiskState, SystemMode
from observability.metrics import FEED_STALE_BLOCKS
from risk_engine.canonical_sizing import compute_canonical_notional
from risk_engine.signing import sign_order_intent


class RiskEngine:
    def __init__(self, settings: AppSettings) -> None:
        self._settings = settings
        self._peak_equity = 100_000.0
        self._current_equity = 100_000.0

    def update_equity(self, equity: float) -> None:
        self._current_equity = equity
        self._peak_equity = max(self._peak_equity, equity)

    @property
    def current_equity(self) -> float:
        """Mark-to-market equity for policy / portfolio sizing (spec pipeline mode)."""
        return self._current_equity

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
        feed_last_message_at: datetime | None = None,
        product_tradable: bool = True,
        position_signed_qty: Decimal | None = None,
        available_cash_usd: float | None = None,
        portfolio_equity_usd: float | None = None,
    ) -> tuple[TradeAction | None, RiskState]:
        """Return None if blocked; otherwise TradeAction for execution layer.

        Precedence (first match wins — see docs/architecture/risk_precedence.md):
        1) Feed disconnected/stale (feed_last_message_at age)
        2) Market data timestamp stale
        3) Spread too wide
        4) Drawdown limit
        5) No proposal / mode blocks / exposure / reduce-only
        """
        now = datetime.now(UTC)
        if feed_last_message_at is not None:
            flm = (
                feed_last_message_at
                if feed_last_message_at.tzinfo
                else feed_last_message_at.replace(tzinfo=UTC)
            ).astimezone(UTC)
            feed_age = abs((now - flm).total_seconds())
            risk = risk.model_copy(update={"data_age_seconds": feed_age, "spread_bps": spread_bps})
            if feed_age > self._settings.risk_stale_data_seconds:
                FEED_STALE_BLOCKS.inc()
                return None, risk

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

        if not product_tradable:
            return None, risk

        mode = risk.mode
        if mode == SystemMode.MAINTENANCE:
            return None, risk

        pos = position_signed_qty if position_signed_qty is not None else Decimal(0)

        if mode == SystemMode.FLATTEN_ALL:
            if pos == 0:
                return None, risk
            qty_close = abs(pos)
            side_close = OrderSide.SELL if pos > 0 else OrderSide.BUY
            rid = proposal.route_id if proposal else RouteId.NO_TRADE
            return (
                TradeAction(
                    symbol=symbol,
                    side=side_close.value,
                    quantity=qty_close,
                    order_type="market",
                    limit_price=None,
                    stop_price=None,
                    time_in_force="gtc",
                    route_id=rid,
                ),
                risk,
            )

        if proposal is None:
            return None, risk

        if mode == SystemMode.PAUSE_NEW_ENTRIES:
            return None, risk

        if mode == SystemMode.REDUCE_ONLY:
            if pos == 0:
                return None, risk
            if pos > 0 and proposal.direction > 0:
                return None, risk
            if pos < 0 and proposal.direction < 0:
                return None, risk

        eq = float(portfolio_equity_usd) if portfolio_equity_usd is not None else self._current_equity
        cn = compute_canonical_notional(
            proposal,
            risk,
            self._settings,
            mid_price=mid_price,
            spread_bps=spread_bps,
            position_signed_qty=position_signed_qty,
            current_total_exposure_usd=current_total_exposure_usd,
            portfolio_equity_usd=eq,
        )
        notional = cn.final_notional_usd
        risk = risk.model_copy(
            update={"last_risk_sizing": cn.diagnostics.model_dump()},
        )
        if notional <= 0:
            return None, risk

        qty = Decimal(str(round(notional / max(mid_price, 1e-12), 8)))
        if qty <= 0:
            return None, risk

        side = OrderSide.BUY if proposal.direction > 0 else OrderSide.SELL
        if (
            side == OrderSide.BUY
            and available_cash_usd is not None
            and notional > float(available_cash_usd)
        ):
            return None, risk

        if mode == SystemMode.REDUCE_ONLY and pos != 0:
            if pos > 0 and side == OrderSide.SELL:
                qty = min(qty, pos)
            elif pos < 0 and side == OrderSide.BUY:
                qty = min(qty, abs(pos))
            if qty <= 0:
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

    def to_order_intent(self, action: TradeAction, *, sign: bool | None = None) -> OrderIntent:
        intent = OrderIntent(
            symbol=action.symbol,
            side=OrderSide(action.side),
            quantity=action.quantity,
            order_type=OrderType.MARKET if action.order_type == "market" else OrderType.LIMIT,
            limit_price=action.limit_price,
            stop_price=action.stop_price,
            time_in_force=TimeInForce.GTC,
            metadata={"route_id": action.route_id.value},
        )
        do_sign = sign if sign is not None else True
        if not do_sign:
            return intent
        secret = (
            self._settings.risk_signing_secret.get_secret_value()
            if self._settings.risk_signing_secret
            else None
        )
        if secret:
            return sign_order_intent(intent, secret)
        return intent
