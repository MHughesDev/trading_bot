"""Replay historical bars through the same decision + risk path as live."""

from __future__ import annotations

from datetime import datetime
from decimal import Decimal
from typing import Any

import polars as pl

from app.config.settings import load_settings
from app.contracts.risk import RiskState
from backtesting.execution_params import BacktestExecutionParams
from backtesting.portfolio import PortfolioTracker
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
) -> list[dict]:
    """
    Walk OHLCV bars; same `enrich_bars_last_row` + `run_decision_tick` as live (cumulative window).

    When ``track_portfolio`` is True, applies simulated slippage + fees from ``execution_params``.
    If ``execution_params`` is omitted, loads defaults from ``AppSettings`` (same YAML as live).
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
        regime, fc, route, proposal, trade_action, risk = run_decision_tick(
            symbol=symbol,
            feature_row=feats,
            spread_bps=spread_bps,
            risk_state=risk,
            pipeline=pipeline,
            risk_engine=risk_engine,
            mid_price=mid,
            data_timestamp=dt,
            position_signed_qty=pos,
            available_cash_usd=avail,
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

        row_dict: dict = {
            "timestamp": ts,
            "route": route.route_id.value,
            "regime": regime.semantic.value,
            "trade": executed.model_dump() if executed else None,
            "solvency_blocked": solvency_blocked,
        }
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

            regime, fc, route, proposal, trade_action, risk = run_decision_tick(
                symbol=symbol,
                feature_row=feats,
                spread_bps=sp,
                risk_state=risk,
                pipeline=pipeline,
                risk_engine=risk_engine,
                mid_price=mid,
                data_timestamp=dt,
                position_signed_qty=pos,
                available_cash_usd=avail_m,
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

            one: dict[str, Any] = {
                "route": route.route_id.value,
                "regime": regime.semantic.value,
                "trade": executed.model_dump() if executed else None,
                "solvency_blocked": solvency_blocked,
            }
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
