"""Map route + forecast to ActionProposal (signals, not raw orders)."""

from __future__ import annotations

from app.contracts.decisions import ActionProposal, RouteId
from app.contracts.forecast import ForecastOutput


def propose_action(
    symbol: str,
    route: RouteId,
    forecast: ForecastOutput,
) -> ActionProposal | None:
    if route == RouteId.NO_TRADE:
        return None
    direction = 1 if forecast.returns_5 > 0 else -1 if forecast.returns_5 < 0 else 0
    if direction == 0:
        return None
    size_map = {
        RouteId.SCALPING: 0.1,
        RouteId.INTRADAY: 0.2,
        RouteId.SWING: 0.35,
        RouteId.CARRY: 0.15,
    }
    stop_pct = max(0.002, min(0.05, forecast.volatility * 2.0))
    return ActionProposal(
        symbol=symbol,
        route_id=route,
        direction=direction,
        size_fraction=size_map.get(route, 0.1),
        stop_distance_pct=stop_pct,
        order_type="market",
        expiry_seconds=3600 if route == RouteId.SWING else 900,
    )
