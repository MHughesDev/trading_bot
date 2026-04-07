from app.config.settings import AppSettings
from app.contracts.forecast import ForecastOutput
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState, SystemMode
from models.routing.route_selector import DeterministicRouteSelector


def test_route_selector_uses_config_thresholds():
    settings = AppSettings(
        routing_spread_trade_max_bps=5.0,
        routing_forecast_strength_min=0.001,
    )
    sel = DeterministicRouteSelector(settings)
    regime = RegimeOutput(
        state_index=0,
        semantic=SemanticRegime.BULL,
        probabilities=[0.25, 0.25, 0.25, 0.25],
        confidence=0.5,
    )
    fc = ForecastOutput(
        returns_1=0.01,
        returns_3=0.01,
        returns_5=0.01,
        returns_15=0.01,
        volatility=0.02,
        uncertainty=0.5,
    )
    risk = RiskState(mode=SystemMode.RUNNING)
    out = sel.decide("BTC-USD", fc, regime, spread_bps=10.0, risk=risk)
    assert out.route_id.value == "NO_TRADE"


def test_execution_router_rejects_wrong_adapter():
    from execution.router import get_execution_adapter

    s = AppSettings(execution_mode="paper", execution_paper_adapter="wrong")
    try:
        get_execution_adapter(s)
        raise AssertionError("expected ValueError")
    except ValueError as e:
        assert "alpaca" in str(e).lower()
