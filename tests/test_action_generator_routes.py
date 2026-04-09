"""Per-route action proposal coverage."""

from __future__ import annotations

import pytest

from app.contracts.decisions import RouteId
from app.contracts.forecast import ForecastOutput
from decision_engine.action_generator import propose_action


def _fc(returns_5: float) -> ForecastOutput:
    return ForecastOutput(
        returns_1=returns_5,
        returns_3=returns_5,
        returns_5=returns_5,
        returns_15=returns_5,
        volatility=0.02,
        uncertainty=0.1,
    )


@pytest.mark.parametrize(
    "route,ret,size",
    [
        (RouteId.SCALPING, 0.01, 0.1),
        (RouteId.INTRADAY, -0.01, 0.2),
        (RouteId.SWING, 0.02, 0.35),
    ],
)
def test_propose_action_per_route(route: RouteId, ret: float, size: float) -> None:
    a = propose_action("BTC-USD", route, _fc(ret))
    assert a is not None
    assert a.route_id == route
    assert a.size_fraction == size
    assert a.direction == (1 if ret > 0 else -1)


def test_no_trade_route_returns_none() -> None:
    assert propose_action("BTC-USD", RouteId.NO_TRADE, _fc(0.01)) is None


def test_flat_forecast_returns_none() -> None:
    fc = ForecastOutput(
        returns_1=0, returns_3=0, returns_5=0, returns_15=0, volatility=0.01, uncertainty=0.1
    )
    assert propose_action("BTC-USD", RouteId.SCALPING, fc) is None
