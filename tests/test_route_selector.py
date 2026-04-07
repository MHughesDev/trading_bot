from __future__ import annotations

from app.contracts.common import RouteId, SemanticRegime
from app.contracts.models import ForecastOutput, RegimeOutput
from models.routing.selector import DeterministicRouteSelector


def test_route_selector_can_choose_non_no_trade_route() -> None:
    selector = DeterministicRouteSelector(no_trade_threshold=0.1)
    forecast = ForecastOutput(
        symbol="BTC-USD",
        horizon_returns={1: 0.008, 3: 0.006, 5: 0.005, 15: 0.004},
        volatility_estimate=0.01,
        confidence=0.8,
        uncertainty=0.2,
    )
    regime = RegimeOutput(
        symbol="BTC-USD",
        raw_state=0,
        semantic_state=SemanticRegime.BULL,
        probabilities=[0.7, 0.1, 0.1, 0.1],
        confidence=0.7,
    )
    decision, ranking = selector.select(
        forecast=forecast,
        regime=regime,
        spread_bps=3.0,
        risk_pressure=0.1,
    )
    assert decision.route_id in {RouteId.SCALPING, RouteId.INTRADAY, RouteId.SWING}
    assert len(ranking) == 3
    assert decision.ranking
