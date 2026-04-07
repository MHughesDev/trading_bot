from datetime import UTC, datetime
from decimal import Decimal

from app.config.settings import AppSettings
from app.contracts.decisions import ActionProposal, RouteId
from app.contracts.risk import RiskState, SystemMode
from risk_engine.engine import RiskEngine


def test_flatten_all_emits_close_long():
    eng = RiskEngine(AppSettings())
    risk = RiskState(mode=SystemMode.FLATTEN_ALL)
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
        position_signed_qty=Decimal("0.01"),
    )
    assert trade is not None
    assert trade.side == "sell"
    assert trade.quantity == Decimal("0.01")


def test_flatten_all_zero_position():
    eng = RiskEngine(AppSettings())
    risk = RiskState(mode=SystemMode.FLATTEN_ALL)
    trade, _ = eng.evaluate(
        "BTC-USD",
        None,
        risk,
        mid_price=50_000.0,
        spread_bps=1.0,
        data_timestamp=datetime.now(UTC),
        position_signed_qty=Decimal(0),
    )
    assert trade is None


def test_reduce_only_allows_sell_when_long():
    eng = RiskEngine(AppSettings())
    risk = RiskState(mode=SystemMode.REDUCE_ONLY)
    prop = ActionProposal(
        symbol="BTC-USD",
        route_id=RouteId.SCALPING,
        direction=-1,
        size_fraction=0.5,
        stop_distance_pct=0.01,
    )
    trade, _ = eng.evaluate(
        "BTC-USD",
        prop,
        risk,
        mid_price=50_000.0,
        spread_bps=1.0,
        data_timestamp=datetime.now(UTC),
        position_signed_qty=Decimal("0.02"),
    )
    assert trade is not None
    assert trade.side == "sell"
    assert trade.quantity <= Decimal("0.02")


def test_reduce_only_blocks_buy_when_long():
    eng = RiskEngine(AppSettings())
    risk = RiskState(mode=SystemMode.REDUCE_ONLY)
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
        position_signed_qty=Decimal("0.02"),
    )
    assert trade is None
