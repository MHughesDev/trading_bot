"""Tests for backtesting.metrics and backtesting.trade_ledger."""

from __future__ import annotations

from datetime import UTC, datetime
from decimal import Decimal

import polars as pl

from app.config.settings import AppSettings
from backtesting.execution_params import BacktestExecutionParams
from backtesting.metrics import (
    annualization_factor,
    compute_backtest_metrics,
    extract_equity_curve,
    max_drawdown,
    periodic_returns,
    sharpe_ratio,
)
from backtesting.replay import replay_decisions
from backtesting.trade_ledger import build_trade_ledger, fills_from_rows, summarize_trades
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine


def _buy(ts, qty, price, fee="0.0"):
    return {
        "timestamp": ts,
        "trade": {"side": "buy", "quantity": Decimal(str(qty)), "order_type": "market"},
        "fill_price": float(price),
        "fee_paid": fee,
    }


def _sell(ts, qty, price, fee="0.0"):
    return {
        "timestamp": ts,
        "trade": {"side": "sell", "quantity": Decimal(str(qty)), "order_type": "market"},
        "fill_price": float(price),
        "fee_paid": fee,
    }


def test_max_drawdown_simple() -> None:
    assert max_drawdown([100.0, 120.0, 90.0, 110.0]) == (120.0 - 90.0) / 120.0
    assert max_drawdown([100.0, 101.0, 102.0]) == 0.0


def test_periodic_returns() -> None:
    rets = periodic_returns([100.0, 110.0, 99.0])
    assert rets[0] == 0.1
    assert abs(rets[1] - (-0.1)) < 1e-12


def test_sharpe_zero_when_flat() -> None:
    assert sharpe_ratio([0.0, 0.0, 0.0], periods_per_year=525600.0) == 0.0


def test_annualization_factor_minute_bars() -> None:
    # 60s bars on a 24/7 calendar => 525,600 bars/year.
    assert annualization_factor(60.0) == 365 * 24 * 60


def test_trade_ledger_long_round_trip() -> None:
    ts = datetime(2025, 1, 1, tzinfo=UTC)
    rows = [_buy(ts, 2, 100.0, fee="0.2"), _sell(ts, 2, 110.0, fee="0.22")]
    records = build_trade_ledger(rows, symbol="BTC-USD")
    assert len(records) == 1
    rec = records[0]
    assert rec.direction == "long"
    assert rec.quantity == 2.0
    assert rec.entry_price == 100.0
    assert rec.exit_price == 110.0
    assert rec.gross_pnl == 20.0
    assert abs(rec.fees - 0.42) < 1e-9
    assert abs(rec.net_pnl - 19.58) < 1e-9


def test_trade_ledger_reversal_emits_two_trades() -> None:
    ts = datetime(2025, 1, 1, tzinfo=UTC)
    rows = [
        _buy(ts, 3, 100.0),   # open long 3
        _sell(ts, 5, 120.0),  # close long 3 (+60), open short 2
        _buy(ts, 2, 110.0),   # close short 2 (+20)
    ]
    records = build_trade_ledger(rows, symbol="BTC-USD")
    assert len(records) == 2
    assert records[0].direction == "long"
    assert records[0].gross_pnl == 60.0
    assert records[1].direction == "short"
    assert records[1].gross_pnl == 20.0


def test_summarize_trades_win_rate_and_profit_factor() -> None:
    ts = datetime(2025, 1, 1, tzinfo=UTC)
    rows = [
        _buy(ts, 1, 100.0),
        _sell(ts, 1, 110.0),  # +10 win
        _buy(ts, 1, 100.0),
        _sell(ts, 1, 95.0),   # -5 loss
    ]
    records = build_trade_ledger(rows, symbol="BTC-USD")
    stats = summarize_trades(records)
    assert stats.num_trades == 2
    assert stats.num_wins == 1
    assert stats.num_losses == 1
    assert stats.win_rate == 0.5
    assert abs(stats.gross_profit - 10.0) < 1e-9
    assert abs(stats.gross_loss + 5.0) < 1e-9
    assert abs(stats.profit_factor - 2.0) < 1e-9
    assert abs(stats.net_pnl - 5.0) < 1e-9


def test_extract_equity_curve_single_and_multi() -> None:
    single = [{"equity_mark": "100"}, {"equity_mark": "101"}]
    _, eq = extract_equity_curve(single)
    assert eq == [100.0, 101.0]
    multi = [{"portfolio_equity_mark": "200"}, {"portfolio_equity_mark": "190"}]
    _, eq2 = extract_equity_curve(multi)
    assert eq2 == [200.0, 190.0]


def test_compute_metrics_on_synthetic_equity() -> None:
    rows = [
        {"timestamp": i, "equity_mark": str(v)}
        for i, v in enumerate([100.0, 102.0, 101.0, 105.0])
    ]
    m = compute_backtest_metrics(rows, bar_interval_seconds=60.0)
    assert m.n_bars == 4
    assert m.initial_equity == 100.0
    assert m.final_equity == 105.0
    assert abs(m.total_return - 0.05) < 1e-9
    assert abs(m.cumulative_pnl - 5.0) < 1e-9
    assert m.max_drawdown == (102.0 - 101.0) / 102.0
    assert m.periods_per_year == 365 * 24 * 60


def test_fills_from_rows_ignores_rows_without_trade() -> None:
    ts = datetime(2025, 1, 1, tzinfo=UTC)
    rows = [{"timestamp": ts, "trade": None}, _buy(ts, 1, 100.0)]
    fills = fills_from_rows(rows, symbol="BTC-USD")
    assert len(fills) == 1
    assert fills[0].side == "buy"


def test_metrics_on_real_replay_run() -> None:
    """End-to-end: replay produces rows, metrics computes without error."""
    bars = []
    for i in range(40):
        t = datetime(2025, 1, 1, 0, i, 0, tzinfo=UTC)
        p = 100.0 + (i % 7) * 0.5
        bars.append({"timestamp": t, "open": p, "high": p + 0.2, "low": p - 0.2, "close": p, "volume": 5.0})
    df = pl.DataFrame(bars)
    out = replay_decisions(
        df,
        DecisionPipeline(),
        RiskEngine(AppSettings()),
        symbol="BTC-USD",
        spread_bps=5.0,
        track_portfolio=True,
        execution_params=BacktestExecutionParams(
            slippage_bps=5.0, fee_bps=10.0, slippage_noise_bps=0.0,
            rng_seed=7, initial_cash=Decimal("100000"),
        ),
    )
    m = compute_backtest_metrics(out, bar_interval_seconds=60.0, symbol="BTC-USD")
    assert m.n_bars == len(out)
    assert m.initial_equity > 0
    # to_dict round-trips for reporting/JSON.
    d = m.to_dict()
    assert "sharpe_ratio" in d and "trade_stats" in d
    assert "win_rate" in d["trade_stats"]
