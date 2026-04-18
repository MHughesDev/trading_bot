"""Shared replay step: fault injection, events, deterministic tick (FB-CAN-009)."""

from __future__ import annotations

from datetime import datetime
from decimal import Decimal
from typing import Any

from app.config.settings import load_settings
from app.contracts.replay_events import ReplayRunContract
from backtesting.fault_injection import apply_fault_injection
from backtesting.replay_events import (
    build_decision_output_event,
    build_execution_feedback_event,
    build_fault_injection_event,
    build_market_snapshot_event,
    build_safety_snapshot_event,
    build_structural_signal_event,
)
from backtesting.replay_helpers import (
    execution_profile_fill_ratio,
    remaining_edge_and_exec_confidence_for_partial_fill,
)
from execution.partial_fill_reconcile import reconcile_partial_fill_record
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine


def run_one_replay_step(
    *,
    symbol: str,
    feats: dict[str, float],
    spread_bps: float,
    dt: datetime | None,
    mid: float,
    risk: Any,
    pipeline: DecisionPipeline,
    risk_engine: RiskEngine,
    pos: Decimal | None,
    avail: float | None,
    eq_usd: float | None,
    contract: ReplayRunContract,
    fault_profile: dict[str, Any],
    collect_events: bool,
    events_out: list[dict[str, Any]] | None,
    execution_profile: str | None = None,
    execution_feedback_state: dict[str, dict[str, float]] | None = None,
) -> tuple[Any, Any, Any, Any, Any, Any]:
    """Apply fault injection, run deterministic tick, optionally append canonical events."""
    fp_in = dict(fault_profile or {})
    feats2, sp2, dt2, fault_reasons = apply_fault_injection(
        feature_row=feats,
        spread_bps=spread_bps,
        data_timestamp=dt,
        profile=fp_in,
    )
    prof = execution_profile or contract.execution_model_profile
    fill_ratio = execution_profile_fill_ratio(prof)

    # Import from `backtesting.replay` so tests can monkeypatch `run_decision_tick`.
    from backtesting import replay as replay_mod

    regime, fc, route, proposal, trade_action, risk_out = replay_mod.run_decision_tick(
        symbol=symbol,
        feature_row=feats2,
        spread_bps=sp2,
        risk_state=risk,
        pipeline=pipeline,
        risk_engine=risk_engine,
        mid_price=mid,
        data_timestamp=dt2,
        position_signed_qty=pos,
        available_cash_usd=avail,
        portfolio_equity_usd=eq_usd,
        replay_deterministic=True,
        execution_feedback_state=execution_feedback_state,
    )

    if collect_events and events_out is not None:
        rid = contract.replay_run_id
        ts = dt2 or dt
        cfg_v = contract.config_version
        logic_v = contract.logic_version
        events_out.append(
            build_market_snapshot_event(
                replay_run_id=rid,
                symbol=symbol,
                timestamp=ts,
                mid_price=mid,
                spread_bps=sp2,
                feature_row=feats2,
            ).model_dump(mode="json")
        )
        events_out.append(
            build_structural_signal_event(
                replay_run_id=rid,
                symbol=symbol,
                timestamp=ts,
                feature_row=feats2,
            ).model_dump(mode="json")
        )
        events_out.append(
            build_safety_snapshot_event(
                replay_run_id=rid,
                symbol=symbol,
                timestamp=ts,
                regime=regime,
                risk=risk_out,
            ).model_dump(mode="json")
        )
        events_out.append(
            build_decision_output_event(
                replay_run_id=rid,
                symbol=symbol,
                timestamp=ts,
                config_version=cfg_v,
                logic_version=logic_v,
                regime=regime,
                forecast=fc,
                route=route,
                proposal=proposal,
                risk=risk_out,
                forecast_packet=pipeline.last_forecast_packet,
            ).model_dump(mode="json")
        )
        if fault_reasons:
            events_out.append(
                build_fault_injection_event(
                    replay_run_id=rid,
                    symbol=symbol,
                    timestamp=ts,
                    reasons=fault_reasons,
                    profile=fp_in,
                ).model_dump(mode="json")
            )
        if trade_action is not None:
            ec = float(getattr(risk_out, "risk_execution_confidence", None) or 0.72)
            pfr: dict[str, Any] | None = None
            if fill_ratio < 1.0 - 1e-12:
                rem_edge, ec_pf = remaining_edge_and_exec_confidence_for_partial_fill(risk_out)
                pfr = reconcile_partial_fill_record(
                    intended_qty=float(trade_action.quantity),
                    fill_ratio=fill_ratio,
                    remaining_edge=rem_edge,
                    execution_confidence_realized=ec_pf,
                    settings=load_settings(),
                ).model_dump(mode="json")
            events_out.append(
                build_execution_feedback_event(
                    replay_run_id=rid,
                    symbol=symbol,
                    timestamp=ts,
                    simulated_fill_price=mid,
                    simulated_fill_ratio=fill_ratio,
                    simulated_latency_ms=45.0,
                    execution_confidence_realized=min(1.0, max(0.0, ec)),
                    profile=str(prof),
                    partial_fill_reconciliation=pfr,
                ).model_dump(mode="json")
            )

    return regime, fc, route, proposal, trade_action, risk_out
