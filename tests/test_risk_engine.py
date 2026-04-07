from __future__ import annotations

from datetime import UTC, datetime, timedelta

from app.config.settings import RiskSettings
from app.contracts.common import OrderType, RouteId, Side
from app.contracts.decisions import OrderIntent
from app.contracts.state import RuntimeState
from risk_engine.engine import RiskEngine


def _order() -> OrderIntent:
    return OrderIntent(
        symbol="BTC-USD",
        side=Side.BUY,
        quantity=1.0,
        order_type=OrderType.MARKET,
        route_id=RouteId.INTRADAY,
        decision_id="d1",
    )


def test_risk_blocks_stale_data() -> None:
    risk = RiskEngine(RiskSettings(stale_data_seconds=5))
    order = _order()
    state = RuntimeState()

    out = risk.evaluate(
        order=order,
        runtime_state=state,
        spread_bps=2.0,
        last_market_ts=datetime.now(UTC) - timedelta(seconds=30),
        mark_price=50_000.0,
    )
    assert out.approved is False
    assert "stale_data_guard" in out.blocked_by


def test_risk_adjusts_large_notional() -> None:
    risk = RiskEngine(RiskSettings(max_order_notional_usd=1000))
    order = _order()
    state = RuntimeState()

    out = risk.evaluate(
        order=order,
        runtime_state=state,
        spread_bps=2.0,
        last_market_ts=datetime.now(UTC),
        mark_price=50_000.0,
    )
    assert out.approved is True
    assert out.adjusted_quantity is not None
    assert out.adjusted_quantity < order.quantity
