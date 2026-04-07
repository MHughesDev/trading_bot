from __future__ import annotations

from dataclasses import dataclass
from uuid import uuid4

from app.contracts.audit import DecisionTrace
from app.contracts.common import OrderType, RouteId, Side
from app.contracts.decisions import ActionIntent, OrderIntent, RouteDecision
from app.contracts.models import ForecastOutput, RegimeOutput
from models.routing.selector import DeterministicRouteSelector


@dataclass(slots=True)
class ActionGenerator:
    base_notional_usd: float = 1_000.0

    def generate(
        self,
        symbol: str,
        last_price: float,
        route_decision: RouteDecision,
        forecast: ForecastOutput,
    ) -> ActionIntent | None:
        if route_decision.route_id == RouteId.NO_TRADE:
            return None

        short_h = min(forecast.horizon_returns.keys()) if forecast.horizon_returns else 1
        expected = forecast.horizon_returns.get(short_h, 0.0)
        side = Side.BUY if expected >= 0 else Side.SELL

        route_mult = {
            RouteId.SCALPING: 0.75,
            RouteId.INTRADAY: 1.0,
            RouteId.SWING: 1.4,
            RouteId.NO_TRADE: 0.0,
        }[route_decision.route_id]
        conf_mult = 0.5 + route_decision.confidence
        notional = self.base_notional_usd * route_mult * conf_mult
        qty = max(notional / max(last_price, 1e-6), 0.0)
        if qty <= 0:
            return None

        stop_distance = max(forecast.volatility_estimate * last_price * 1.5, last_price * 0.0025)
        expiry = {RouteId.SCALPING: 300, RouteId.INTRADAY: 3600, RouteId.SWING: 24 * 3600}.get(
            route_decision.route_id,
            300,
        )
        order_type = (
            OrderType.MARKET if route_decision.route_id != RouteId.SWING else OrderType.LIMIT
        )

        return ActionIntent(
            symbol=symbol,
            side=side,
            quantity=qty,
            order_type=order_type,
            stop_distance=stop_distance,
            expiry_seconds=expiry,
            metadata={
                "route": route_decision.route_id.value,
                "confidence": route_decision.confidence,
                "expected_return": expected,
            },
        )


@dataclass(slots=True)
class DecisionEngine:
    route_selector: DeterministicRouteSelector
    action_generator: ActionGenerator

    def run(
        self,
        symbol: str,
        last_price: float,
        features: dict[str, float],
        memory_features: dict[str, float],
        forecast: ForecastOutput,
        regime: RegimeOutput,
        spread_bps: float,
        risk_pressure: float,
    ) -> tuple[DecisionTrace, ActionIntent | None]:
        route_decision, route_scores = self.route_selector.select(
            forecast=forecast,
            regime=regime,
            spread_bps=spread_bps,
            risk_pressure=risk_pressure,
        )
        action = self.action_generator.generate(
            symbol=symbol,
            last_price=last_price,
            route_decision=route_decision,
            forecast=forecast,
        )
        score_summary = ",".join(
            f"{score.route_id.value}:{score.score:.3f}" for score in route_scores
        )

        trace = DecisionTrace(
            trace_id=str(uuid4()),
            symbol=symbol,
            features=features,
            memory_features=memory_features,
            forecast=forecast.model_dump(),
            regime=regime.model_dump(),
            route_decision=RouteDecision(
                route_id=route_decision.route_id,
                confidence=route_decision.confidence,
                ranking=route_decision.ranking,
                reasons=[
                    *route_decision.reasons,
                    f"route_scores={{{score_summary}}}",
                ],
            ),
            action_intent=action,
        )
        return trace, action

    def action_to_order_intent(
        self,
        trace_id: str,
        route_id: RouteId,
        action: ActionIntent,
        last_price: float,
    ) -> OrderIntent:
        limit_price = None
        if action.order_type == OrderType.LIMIT:
            # Passive bias: buy slightly below / sell slightly above.
            skew = 0.0015
            limit_price = last_price * (1 - skew if action.side == Side.BUY else 1 + skew)

        stop_price = None
        if action.stop_distance is not None:
            stop_price = (
                last_price - action.stop_distance
                if action.side == Side.BUY
                else last_price + action.stop_distance
            )

        if limit_price is not None:
            limit_price = max(limit_price, 1e-8)
        if stop_price is not None:
            stop_price = max(stop_price, 1e-8)

        return OrderIntent(
            symbol=action.symbol,
            side=action.side,
            quantity=action.quantity,
            order_type=action.order_type,
            limit_price=limit_price,
            stop_price=stop_price,
            route_id=route_id,
            decision_id=trace_id,
        )
