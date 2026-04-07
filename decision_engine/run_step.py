"""
Single decision + risk step shared by live runtime and backtest replay (spec: same path).

Import this module from both `app/runtime/live_service.py` and `backtesting/replay.py`
so drift is visible in one place.
"""

from __future__ import annotations

from datetime import datetime
from decimal import Decimal

from app.contracts.decisions import ActionProposal, RouteDecision, TradeAction
from app.contracts.forecast import ForecastOutput
from app.contracts.regime import RegimeOutput
from app.contracts.risk import RiskState
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine


def run_decision_tick(
    *,
    symbol: str,
    feature_row: dict[str, float],
    spread_bps: float,
    risk_state: RiskState,
    pipeline: DecisionPipeline,
    risk_engine: RiskEngine,
    mid_price: float,
    data_timestamp: datetime | None,
    current_total_exposure_usd: float = 0.0,
    feed_last_message_at: datetime | None = None,
    product_tradable: bool = True,
    position_signed_qty: Decimal | None = None,
) -> tuple[RegimeOutput, ForecastOutput, RouteDecision, ActionProposal | None, TradeAction | None, RiskState]:
    regime, fc, route, proposal = pipeline.step(symbol, feature_row, spread_bps, risk_state)
    trade, risk_state = risk_engine.evaluate(
        symbol,
        proposal,
        risk_state,
        mid_price=mid_price,
        spread_bps=spread_bps,
        data_timestamp=data_timestamp,
        current_total_exposure_usd=current_total_exposure_usd,
        feed_last_message_at=feed_last_message_at,
        product_tradable=product_tradable,
        position_signed_qty=position_signed_qty,
    )
    return regime, fc, route, proposal, trade, risk_state
