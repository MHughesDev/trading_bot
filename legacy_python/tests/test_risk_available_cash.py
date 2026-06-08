"""Optional available_cash in RiskEngine (FB-B1)."""

from __future__ import annotations

from datetime import UTC, datetime

from app.config.settings import AppSettings
from app.contracts.decisions import ActionProposal, RouteId
from app.contracts.risk import RiskState
from risk_engine.engine import RiskEngine


def test_risk_blocks_buy_when_cash_insufficient() -> None:
    settings = AppSettings()
    eng = RiskEngine(settings)
    risk = RiskState()
    prop = ActionProposal(
        symbol="BTC-USD",
        route_id=RouteId.SCALPING,
        direction=1,
        size_fraction=0.5,
        stop_distance_pct=0.01,
    )
    now = datetime.now(UTC)
    trade, _ = eng.evaluate(
        "BTC-USD",
        prop,
        risk,
        mid_price=50_000.0,
        spread_bps=1.0,
        data_timestamp=now,
        available_cash_usd=100.0,
    )
    assert trade is None
