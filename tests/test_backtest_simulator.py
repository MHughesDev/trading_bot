"""Backtest simulator: fees, slippage, seeded noise, portfolio replay."""

from __future__ import annotations

from datetime import UTC, datetime
from decimal import Decimal

import polars as pl

from app.config.settings import AppSettings
from backtesting.execution_params import BacktestExecutionParams
from backtesting.replay import replay_decisions
from backtesting.simulator import (
    apply_slippage,
    cash_delta_for_trade,
    fee_on_notional,
    fill_price_with_slippage,
    make_replay_rng,
)
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine


def test_apply_slippage_buy_worse() -> None:
    p = apply_slippage(100.0, "buy", 10.0)
    assert p > 100.0


def test_fee_on_notional() -> None:
    assert fee_on_notional(Decimal("1000"), 10.0) == Decimal("1")


def test_cash_delta_buy_includes_fee() -> None:
    cash, fee = cash_delta_for_trade(side="buy", qty=Decimal("1"), fill_price=100.0, fee_bps=10.0)
    assert fee == Decimal("0.1")
    assert cash == Decimal("-100.1")


def test_fill_price_noise_reproducible_seed() -> None:
    r1 = make_replay_rng(42)
    r2 = make_replay_rng(42)
    a = fill_price_with_slippage(100.0, "buy", slippage_bps=5.0, slippage_noise_bps=2.0, rng=r1)
    b = fill_price_with_slippage(100.0, "buy", slippage_bps=5.0, slippage_noise_bps=2.0, rng=r2)
    assert a == b


def test_replay_track_portfolio_adds_columns():
    rows = []
    for i in range(20):
        t = datetime(2025, 1, 1, 0, i, 0, tzinfo=UTC)
        p = 100.0 + i * 0.01
        rows.append({"timestamp": t, "open": p, "high": p, "low": p, "close": p, "volume": 1.0})
    df = pl.DataFrame(rows)
    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    out = replay_decisions(
        df,
        pipe,
        eng,
        symbol="BTC-USD",
        spread_bps=5.0,
        track_portfolio=True,
        execution_params=BacktestExecutionParams(
            slippage_bps=5.0,
            fee_bps=10.0,
            slippage_noise_bps=0.0,
            rng_seed=123,
            initial_cash=Decimal("100000"),
        ),
    )
    assert out
    last = out[-1]
    assert "portfolio_cash" in last
    assert "equity_mark" in last
