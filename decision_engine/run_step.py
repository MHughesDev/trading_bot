"""
Single decision + risk step shared by live runtime and backtest replay (spec: same path).

Import this module from both `app/runtime/live_service.py` and `backtesting/replay.py`
so drift is visible in one place.
"""

from __future__ import annotations

import time
from datetime import datetime
from decimal import Decimal

from app.contracts.decisions import ActionProposal, RouteDecision, RouteId, TradeAction
from app.contracts.forecast import ForecastOutput
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState
from app.runtime.system_power import is_on, sync_from_disk
from decision_engine.pipeline import DecisionPipeline
from observability.metrics import DECISION_LATENCY
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
    available_cash_usd: float | None = None,
    portfolio_equity_usd: float | None = None,
) -> tuple[RegimeOutput, ForecastOutput, RouteDecision, ActionProposal | None, TradeAction | None, RiskState]:
    t0 = time.perf_counter()
    sync_from_disk()
    eq = portfolio_equity_usd
    if eq is None:
        eq = risk_engine.current_equity
    if not is_on():
        regime = RegimeOutput(
            state_index=0,
            semantic=SemanticRegime.SIDEWAYS,
            probabilities=[1.0, 0.0, 0.0, 0.0],
            confidence=0.0,
        )
        fc = ForecastOutput(
            returns_1=0.0,
            returns_3=0.0,
            returns_5=0.0,
            returns_15=0.0,
            volatility=0.0,
            uncertainty=1.0,
        )
        route = RouteDecision(route_id=RouteId.NO_TRADE, confidence=0.0, ranking=[])
        proposal = None
        trade, risk_state = risk_engine.evaluate(
            symbol,
            proposal,
            risk_state,
            mid_price=mid_price,
            spread_bps=spread_bps,
            data_timestamp=data_timestamp,
            current_total_exposure_usd=current_total_exposure_usd,
            feed_last_message_at=feed_last_message_at,
            product_tradable=False,
            position_signed_qty=position_signed_qty,
            available_cash_usd=available_cash_usd,
            portfolio_equity_usd=eq,
        )
        DECISION_LATENCY.observe(time.perf_counter() - t0)
        return regime, fc, route, proposal, trade, risk_state

    regime, fc, route, proposal = pipeline.step(
        symbol,
        feature_row,
        spread_bps,
        risk_state,
        mid_price=mid_price,
        portfolio_equity_usd=eq,
        position_signed_qty=position_signed_qty,
    )
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
        available_cash_usd=available_cash_usd,
        portfolio_equity_usd=eq,
    )
    DECISION_LATENCY.observe(time.perf_counter() - t0)
    return regime, fc, route, proposal, trade, risk_state
