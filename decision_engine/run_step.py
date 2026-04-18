"""
Single decision + risk step shared by live runtime and backtest replay (spec: same path).

Import this module from both `app/runtime/live_service.py` and `backtesting/replay.py`
so drift is visible in one place.

Canonical sequence (FB-CAN-029): ``pipeline.step`` runs normalize → forecast →
:func:`decision_engine.canonical_orchestrator.run_canonical_decision_sequence_after_forecast`
(structure → state → trigger → auction → carry); this function then runs ``risk_engine.evaluate``
(execution intent sizing / gating). Downstream: ``live_service`` builds execution guidance and submits orders.
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
from decision_engine.decision_record import build_decision_record, set_last_decision_record
from decision_engine.pipeline import DecisionPipeline
from observability.canonical_metrics import maybe_set_config_version_from_engine, record_canonical_post_tick
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
    replay_deterministic: bool = False,
    execution_feedback_state: dict[str, dict[str, float]] | None = None,
) -> tuple[RegimeOutput, ForecastOutput, RouteDecision, ActionProposal | None, TradeAction | None, RiskState]:
    t0 = time.perf_counter()
    if not replay_deterministic:
        sync_from_disk()
    eq = portfolio_equity_usd
    if eq is None:
        eq = risk_engine.current_equity
    if not replay_deterministic and not is_on():
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
        maybe_set_config_version_from_engine(risk_engine)
        risk_state = risk_state.model_copy(
            update={
                "last_decision_record": {
                    "schema_version": 1,
                    "outcome": "no_trade",
                    "no_trade": {
                        "event_id": "power-off",
                        "no_trade_reason_codes": ["system_power_off"],
                    },
                }
            },
        )
        record_canonical_post_tick(
            symbol=symbol,
            regime=regime,
            risk=risk_state,
            forecast_packet=None,
            carry_sleeve=getattr(risk_state, "carry_sleeve_last", None),
            feature_row=feature_row,
        )
        return regime, fc, route, proposal, trade, risk_state

    regime, fc, route, proposal, risk_state = pipeline.step(
        symbol,
        feature_row,
        spread_bps,
        risk_state,
        mid_price=mid_price,
        portfolio_equity_usd=eq,
        current_total_exposure_usd=current_total_exposure_usd,
        position_signed_qty=position_signed_qty,
        data_timestamp=data_timestamp,
        feed_last_message_at=feed_last_message_at,
        now_ref=data_timestamp,
        product_tradable=product_tradable,
        execution_feedback_state=execution_feedback_state,
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
    dr = build_decision_record(
        symbol=symbol,
        data_timestamp=data_timestamp,
        settings=risk_engine._settings,
        regime=regime,
        forecast=fc,
        route=route,
        proposal=proposal,
        risk=risk_state,
        forecast_packet=pipeline.last_forecast_packet,
        trade=trade,
        feature_row=feature_row,
        mid_price=mid_price,
    )
    risk_state = risk_state.model_copy(
        update={"last_decision_record": dr.model_dump(mode="json")},
    )
    set_last_decision_record(dr)
    if pipeline.last_forecast_packet is not None:
        pipeline.last_forecast_packet.forecast_diagnostics["decision_record"] = dr.model_dump(
            mode="json"
        )
    DECISION_LATENCY.observe(time.perf_counter() - t0)
    maybe_set_config_version_from_engine(risk_engine)
    record_canonical_post_tick(
        symbol=symbol,
        regime=regime,
        risk=risk_state,
        forecast_packet=pipeline.last_forecast_packet,
        carry_sleeve=getattr(risk_state, "carry_sleeve_last", None),
        feature_row=feature_row,
    )
    return regime, fc, route, proposal, trade, risk_state
