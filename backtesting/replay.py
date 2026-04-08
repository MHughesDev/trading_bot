"""Replay historical bars through the same decision + risk path as live."""

from __future__ import annotations

from datetime import datetime
from decimal import Decimal

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
    if track_portfolio:
        execp = execution_params or BacktestExecutionParams.from_settings(load_settings())
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
        regime, fc, route, action, trade, risk = run_decision_tick(
            symbol=symbol,
            feature_row=feats,
            spread_bps=spread_bps,
            risk_state=risk,
            pipeline=pipeline,
            risk_engine=risk_engine,
            mid_price=mid,
            data_timestamp=dt,
            position_signed_qty=pos,
        )
        fill_price: float | None = None
        fee_paid: Decimal | None = None
        cash_delta: Decimal | None = None

        if trade is not None and portfolio is not None and execp is not None and rng is not None:
            q = Decimal(str(trade.quantity))
            fill_price = fill_price_with_slippage(
                mid,
                trade.side,
                slippage_bps=execp.slippage_bps,
                slippage_noise_bps=execp.slippage_noise_bps,
                rng=rng,
            )
            cd, fee = cash_delta_for_trade(
                side=trade.side,
                qty=q,
                fill_price=fill_price,
                fee_bps=execp.fee_bps,
            )
            portfolio.apply_trade(symbol, q, trade.side, cd)
            fee_paid = fee
            cash_delta = cd
            if trade.side == "buy":
                pos += q
            else:
                pos -= q
        elif trade is not None:
            q = Decimal(str(trade.quantity))
            if trade.side == "buy":
                pos += q
            else:
                pos -= q

        row_dict: dict = {
            "timestamp": ts,
            "route": route.route_id.value,
            "regime": regime.semantic.value,
            "trade": trade.model_dump() if trade else None,
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
