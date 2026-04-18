"""Replay historical bars through the same decision + risk path as live."""

from __future__ import annotations

from datetime import datetime
from decimal import Decimal
from typing import Any

import polars as pl

from app.config.settings import load_settings
from app.contracts.replay_events import ReplayRunContract
from app.contracts.risk import RiskState
from backtesting.execution_params import BacktestExecutionParams
from backtesting.portfolio import PortfolioTracker
from backtesting.replay_core import run_one_replay_step
from backtesting.replay_helpers import (
    execution_feedback_from_simulated_fill,
    execution_profile_fill_ratio,
)
from backtesting.simulator import (
    cash_delta_for_trade,
    fill_price_with_slippage,
    make_replay_rng,
)
from data_plane.features.pipeline import FeaturePipeline
from decision_engine.feature_frame import enrich_bars_last_row
from decision_engine.pipeline import DecisionPipeline
from decision_engine.run_step import run_decision_tick
from risk_engine.engine import RiskEngine

__all__ = ["replay_decisions", "replay_multi_asset_decisions", "run_decision_tick"]


def replay_decisions(
    bars: pl.DataFrame,
    pipeline: DecisionPipeline,
    risk_engine: RiskEngine,
    *,
    symbol: str,
    spread_bps: float = 5.0,
    feature_pipeline: FeaturePipeline | None = None,
    position_signed_qty: Decimal | None = None,
    execution_params: BacktestExecutionParams | None = None,
    track_portfolio: bool = False,
    replay_contract: ReplayRunContract | None = None,
    emit_canonical_events: bool = False,
    fault_injection_profile: dict[str, Any] | None = None,
) -> list[dict]:
    """
    Walk OHLCV bars; same `enrich_bars_last_row` + `run_decision_tick` as live (cumulative window).

    When ``track_portfolio`` is True, applies simulated slippage + fees from ``execution_params``.
    If ``execution_params`` is omitted, loads defaults from ``AppSettings`` (same YAML as live).

    **FB-CAN-009:** Pass ``replay_contract`` for config/version fields; set ``emit_canonical_events``
    to append per-bar ``canonical_events`` (market, structural, safety, decision, optional fault,
    execution feedback). Uses ``replay_deterministic=True`` so replay does not depend on system power
    disk sync. ``fault_injection_profile`` merges over ``replay_contract.fault_injection_profile``.
    """
    if bars.height == 0:
        return []
    fp = feature_pipeline or FeaturePipeline()
    ohlcv_cols = {"timestamp", "open", "high", "low", "close", "volume"}
    frame = bars.sort("timestamp")
    base_cols = [c for c in frame.columns if c in ohlcv_cols]
    raw = frame.select(base_cols) if base_cols else frame

    risk = RiskState()
    rows_out: list[dict] = []
    pos = position_signed_qty if position_signed_qty is not None else Decimal(0)

    execp: BacktestExecutionParams | None = None
    rng = None
    portfolio: PortfolioTracker | None = None
    app_s = load_settings()
    if track_portfolio:
        execp = execution_params or BacktestExecutionParams.from_settings(app_s)
        rng = make_replay_rng(execp.rng_seed)
        portfolio = PortfolioTracker(execp.initial_cash)

    contract = replay_contract or ReplayRunContract(
        replay_run_id="replay-inline",
        dataset_id="inline",
        instrument_scope=[symbol],
    )
    fault_base = dict(contract.fault_injection_profile)
    if fault_injection_profile:
        fault_base.update(fault_injection_profile)

    exec_feedback_state: dict[str, dict[str, float]] = {}
    prof_name = contract.execution_model_profile

    for row in frame.iter_rows(named=True):
        ts = row.get("timestamp")
        sub = raw.filter(pl.col("timestamp") <= ts) if ts is not None else raw
        feats = enrich_bars_last_row(sub, fp)
        if feats is None:
            feats = {
                k: float(v)
                for k, v in row.items()
                if k != "timestamp" and isinstance(v, (int, float))
            }
        if isinstance(ts, datetime):
            dt = ts
        else:
            dt = None
        mid = float(row.get("close", 1.0))
        avail: float | None = None
        if track_portfolio and portfolio is not None and app_s.backtesting_replay_available_cash:
            avail = float(portfolio.cash)
        eq_usd: float | None = None
        if track_portfolio and portfolio is not None:
            eq_usd = float(portfolio.market_value({symbol: mid}))
        row_events: list[dict[str, Any]] | None = [] if emit_canonical_events else None
        regime, fc, route, proposal, trade_action, risk = run_one_replay_step(
            symbol=symbol,
            feats=feats,
            spread_bps=spread_bps,
            dt=dt,
            mid=mid,
            risk=risk,
            pipeline=pipeline,
            risk_engine=risk_engine,
            pos=pos,
            avail=avail,
            eq_usd=eq_usd,
            contract=contract,
            fault_profile=fault_base,
            collect_events=emit_canonical_events,
            events_out=row_events,
            execution_feedback_state=exec_feedback_state,
        )
        fill_price: float | None = None
        fee_paid: Decimal | None = None
        cash_delta: Decimal | None = None
        solvency_blocked = False
        executed = trade_action

        if trade_action is not None and portfolio is not None and execp is not None and rng is not None:
            q = Decimal(str(trade_action.quantity))
            fill_price = fill_price_with_slippage(
                mid,
                trade_action.side,
                slippage_bps=execp.slippage_bps,
                slippage_noise_bps=execp.slippage_noise_bps,
                rng=rng,
            )
            cd, fee = cash_delta_for_trade(
                side=trade_action.side,
                qty=q,
                fill_price=fill_price,
                fee_bps=execp.fee_bps,
            )
            solvency_ok = True
            if execp.enforce_solvency and trade_action.side == "buy" and portfolio.cash + cd < 0:
                solvency_ok = False
                solvency_blocked = True
            if solvency_ok:
                portfolio.apply_trade(symbol, q, trade_action.side, cd)
                fee_paid = fee
                cash_delta = cd
                if trade_action.side == "buy":
                    pos += q
                else:
                    pos -= q
            else:
                executed = None
                fill_price = None
                fee_paid = None
                cash_delta = None
        elif trade_action is not None:
            q = Decimal(str(trade_action.quantity))
            if trade_action.side == "buy":
                pos += q
            else:
                pos -= q

        if executed is not None:
            fill_px = float(fill_price) if fill_price is not None else float(mid)
            fr = execution_profile_fill_ratio(prof_name)
            execution_feedback_from_simulated_fill(
                symbol=symbol,
                mid_price=mid,
                fill_price=fill_px,
                fill_ratio=fr,
                latency_ms=40.0 + 12.0 * (1.0 - fr),
                exec_state=exec_feedback_state,
            )

        row_dict: dict = {
            "timestamp": ts,
            "route": route.route_id.value,
            "regime": regime.semantic.value,
            "trade": executed.model_dump() if executed else None,
            "solvency_blocked": solvency_blocked,
        }
        if emit_canonical_events and row_events is not None:
            row_dict["canonical_events"] = row_events
        if track_portfolio and portfolio is not None:
            row_dict["portfolio_cash"] = str(portfolio.cash)
            row_dict["portfolio_position_qty"] = str(portfolio.positions.get(symbol, Decimal(0)))
            row_dict["fill_price"] = fill_price
            row_dict["fee_paid"] = str(fee_paid) if fee_paid is not None else None
            row_dict["cash_delta"] = str(cash_delta) if cash_delta is not None else None
            row_dict["equity_mark"] = str(portfolio.market_value({symbol: mid}))

        rows_out.append(row_dict)
    return rows_out


def replay_multi_asset_decisions(
    bars_by_symbol: dict[str, pl.DataFrame],
    pipeline: DecisionPipeline,
    risk_engine: RiskEngine,
    *,
    spread_bps_by_symbol: dict[str, float] | None = None,
    default_spread_bps: float = 5.0,
    feature_pipeline: FeaturePipeline | None = None,
    initial_positions: dict[str, Decimal] | None = None,
    execution_params: BacktestExecutionParams | None = None,
    track_portfolio: bool = False,
    replay_contract: ReplayRunContract | None = None,
    emit_canonical_events: bool = False,
    fault_injection_profile: dict[str, Any] | None = None,
) -> list[dict[str, Any]]:
    """
    Multi-symbol replay with **one shared** ``RiskState`` and (when ``track_portfolio``)
    **one** ``PortfolioTracker`` — same ``run_decision_tick`` path as live/replay per bar.

    Per timestamp, symbols are processed in **sorted order** (deterministic). Each symbol
    uses a cumulative OHLCV window up to that timestamp (same as ``replay_decisions``).

    Each output row has ``timestamp``, ``symbols`` (dict of per-symbol payloads), and
    when ``track_portfolio`` is True: ``portfolio_cash``, ``portfolio_equity_mark``,
    ``positions`` (symbol → qty str).
    """
    if not bars_by_symbol:
        return []

    fp = feature_pipeline or FeaturePipeline()
    ohlcv_cols = {"timestamp", "open", "high", "low", "close", "volume"}
    frames: dict[str, pl.DataFrame] = {}
    all_ts: set[Any] = set()
    for sym, df in bars_by_symbol.items():
        if df.height == 0:
            continue
        sorted_df = df.sort("timestamp")
        frames[sym] = sorted_df
        for t in sorted_df["timestamp"].to_list():
            all_ts.add(t)

    if not frames:
        return []

    symbols_sorted = sorted(frames.keys())
    sorted_ts = sorted(all_ts, key=lambda x: x.timestamp() if isinstance(x, datetime) else float(x))

    spread_map = spread_bps_by_symbol or {}
    init_pos = initial_positions or {}
    positions: dict[str, Decimal] = {s: init_pos.get(s, Decimal(0)) for s in symbols_sorted}

    risk = RiskState()
    contract = replay_contract or ReplayRunContract(
        replay_run_id="replay-multi-inline",
        dataset_id="inline",
        instrument_scope=list(symbols_sorted),
    )
    fault_base = dict(contract.fault_injection_profile)
    if fault_injection_profile:
        fault_base.update(fault_injection_profile)
    exec_feedback_state: dict[str, dict[str, float]] = {}
    prof_name = contract.execution_model_profile
    execp: BacktestExecutionParams | None = None
    rng = None
    portfolio: PortfolioTracker | None = None
    app_s = load_settings()
    if track_portfolio:
        execp = execution_params or BacktestExecutionParams.from_settings(app_s)
        rng = make_replay_rng(execp.rng_seed)
        portfolio = PortfolioTracker(execp.initial_cash)

    last_mid: dict[str, float] = {}
    rows_out: list[dict[str, Any]] = []

    for ts in sorted_ts:
        sym_payload: dict[str, Any] = {}
        for symbol in symbols_sorted:
            frame = frames[symbol]
            sub = frame.filter(pl.col("timestamp") <= ts)
            at_ts = sub.filter(pl.col("timestamp") == ts)
            if at_ts.height == 0:
                continue

            row = at_ts.to_dicts()[0]
            base_cols = [c for c in sub.columns if c in ohlcv_cols]
            raw = sub.select(base_cols) if base_cols else sub
            feats = enrich_bars_last_row(raw, fp)
            if feats is None:
                feats = {
                    k: float(v)
                    for k, v in row.items()
                    if k != "timestamp" and isinstance(v, (int, float))
                }
            dt = ts if isinstance(ts, datetime) else None
            mid = float(row.get("close", 1.0))
            last_mid[symbol] = mid
            sp = spread_map.get(symbol, default_spread_bps)
            pos = positions.get(symbol, Decimal(0))

            avail_m: float | None = None
            if track_portfolio and portfolio is not None and app_s.backtesting_replay_available_cash:
                avail_m = float(portfolio.cash)
            eq_m: float | None = None
            if track_portfolio and portfolio is not None:
                px = dict(last_mid)
                px[symbol] = mid
                eq_m = float(portfolio.market_value(px))

            row_events: list[dict[str, Any]] | None = [] if emit_canonical_events else None
            regime, fc, route, proposal, trade_action, risk = run_one_replay_step(
                symbol=symbol,
                feats=feats,
                spread_bps=sp,
                dt=dt,
                mid=mid,
                risk=risk,
                pipeline=pipeline,
                risk_engine=risk_engine,
                pos=pos,
                avail=avail_m,
                eq_usd=eq_m,
                contract=contract,
                fault_profile=fault_base,
                collect_events=emit_canonical_events,
                events_out=row_events,
                execution_feedback_state=exec_feedback_state,
            )

            fill_price: float | None = None
            fee_paid: Decimal | None = None
            cash_delta: Decimal | None = None
            solvency_blocked = False
            executed = trade_action

            if trade_action is not None and portfolio is not None and execp is not None and rng is not None:
                q = Decimal(str(trade_action.quantity))
                fill_price = fill_price_with_slippage(
                    mid,
                    trade_action.side,
                    slippage_bps=execp.slippage_bps,
                    slippage_noise_bps=execp.slippage_noise_bps,
                    rng=rng,
                )
                cd, fee = cash_delta_for_trade(
                    side=trade_action.side,
                    qty=q,
                    fill_price=fill_price,
                    fee_bps=execp.fee_bps,
                )
                solvency_ok = True
                if execp.enforce_solvency and trade_action.side == "buy" and portfolio.cash + cd < 0:
                    solvency_ok = False
                    solvency_blocked = True
                if solvency_ok:
                    portfolio.apply_trade(symbol, q, trade_action.side, cd)
                    fee_paid = fee
                    cash_delta = cd
                    if trade_action.side == "buy":
                        positions[symbol] = positions.get(symbol, Decimal(0)) + q
                    else:
                        positions[symbol] = positions.get(symbol, Decimal(0)) - q
                else:
                    executed = None
                    fill_price = None
                    fee_paid = None
                    cash_delta = None
            elif trade_action is not None:
                q = Decimal(str(trade_action.quantity))
                if trade_action.side == "buy":
                    positions[symbol] = positions.get(symbol, Decimal(0)) + q
                else:
                    positions[symbol] = positions.get(symbol, Decimal(0)) - q

            if executed is not None:
                fill_px = float(fill_price) if fill_price is not None else float(mid)
                fr = execution_profile_fill_ratio(prof_name)
                execution_feedback_from_simulated_fill(
                    symbol=symbol,
                    mid_price=mid,
                    fill_price=fill_px,
                    fill_ratio=fr,
                    latency_ms=40.0 + 12.0 * (1.0 - fr),
                    exec_state=exec_feedback_state,
                )

            one: dict[str, Any] = {
                "route": route.route_id.value,
                "regime": regime.semantic.value,
                "trade": executed.model_dump() if executed else None,
                "solvency_blocked": solvency_blocked,
            }
            if emit_canonical_events and row_events is not None:
                one["canonical_events"] = row_events
            if track_portfolio and portfolio is not None:
                one["fill_price"] = fill_price
                one["fee_paid"] = str(fee_paid) if fee_paid is not None else None
                one["cash_delta"] = str(cash_delta) if cash_delta is not None else None
            sym_payload[symbol] = one

        bundle: dict[str, Any] = {"timestamp": ts, "symbols": sym_payload}
        if track_portfolio and portfolio is not None:
            bundle["portfolio_cash"] = str(portfolio.cash)
            mv_prices = {s: last_mid.get(s, 0.0) for s in portfolio.positions}
            bundle["portfolio_equity_mark"] = str(portfolio.market_value(mv_prices))
            bundle["positions"] = {s: str(q) for s, q in portfolio.positions.items()}
        rows_out.append(bundle)
    return rows_out
