from datetime import UTC, datetime, timedelta

from app.config.settings import AppSettings
from app.contracts.decisions import ActionProposal, RouteId
from app.contracts.risk import RiskState
from risk_engine.engine import RiskEngine


def test_feed_last_message_blocks_before_trade():
    settings = AppSettings(risk_stale_data_seconds=60)
    eng = RiskEngine(settings)
    risk = RiskState()
    old_feed = datetime.now(UTC) - timedelta(seconds=120)
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
        feed_last_message_at=old_feed,
    )
    assert trade is None
