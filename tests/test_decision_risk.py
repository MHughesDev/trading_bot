from datetime import UTC, datetime

from app.config.settings import AppSettings
from app.contracts.risk import RiskState, SystemMode
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine


def test_decision_pipeline_step():
    pipe = DecisionPipeline()
    risk = RiskState()
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    regime, fc, route, action = pipe.step("BTC-USD", feats, spread_bps=5.0, risk=risk)
    assert regime.semantic.value in ("bull", "bear", "volatile", "sideways")
    assert route.route_id.value in ("NO_TRADE", "SCALPING", "INTRADAY", "SWING")


def test_risk_engine_blocks_stale():
    settings = AppSettings()
    eng = RiskEngine(settings)
    risk = RiskState()
    old = datetime(2020, 1, 1, tzinfo=UTC)
    trade, _ = eng.evaluate(
        "BTC-USD",
        None,
        risk,
        mid_price=50_000.0,
        spread_bps=1.0,
        data_timestamp=old,
    )
    assert trade is None


def test_risk_engine_blocks_pause():
    settings = AppSettings()
    eng = RiskEngine(settings)
    risk = RiskState(mode=SystemMode.PAUSE_NEW_ENTRIES)
    from app.contracts.decisions import ActionProposal, RouteId

    prop = ActionProposal(
        symbol="BTC-USD",
        route_id=RouteId.SCALPING,
        direction=1,
        size_fraction=0.1,
        stop_distance_pct=0.01,
    )
    trade, _ = eng.evaluate(
        "BTC-USD",
        prop,
        risk,
        mid_price=50_000.0,
        spread_bps=1.0,
        data_timestamp=datetime.now(UTC),
    )
    assert trade is None
