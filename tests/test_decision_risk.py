from datetime import UTC, datetime

from app.config.settings import AppSettings
from app.contracts.risk import RiskState, SystemMode
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine


def test_pipeline_step_merges_canonical_risk_inputs():
    """RiskState after `step` carries canonical degradation + sizing inputs from state/trigger merge."""
    pipe = DecisionPipeline()
    risk = RiskState()
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = 50_000.0
    feats["volume"] = 1e6
    regime, fc, route, action, risk_out = pipe.step(
        "BTC-USD",
        feats,
        spread_bps=5.0,
        risk=risk,
        mid_price=float(feats.get("close", 50000.0)),
        portfolio_equity_usd=100_000.0,
    )
    assert regime.semantic.value in ("bull", "bear", "volatile", "sideways")
    assert route.route_id.value in ("NO_TRADE", "SCALPING", "INTRADAY", "SWING", "CARRY")
    assert risk_out.canonical_degradation is not None
    assert risk_out.risk_liquidation_mode is not None
    assert risk_out.risk_asymmetry_score is not None
    assert fc.volatility >= 0.0
    assert action is None or action.symbol == "BTC-USD"


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
