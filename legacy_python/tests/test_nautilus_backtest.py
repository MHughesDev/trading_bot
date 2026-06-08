"""Tests for the NautilusTrader backtest wrapper (FB-AP-XXX).

The end-to-end run requires the optional `nautilus_trader` extra; validation that doesn't need
the engine (unknown strategy, empty bars) runs unconditionally so the module stays exercisable
without the optional dependency, mirroring `strategies/registry.py`'s lazy-import contract.
"""

from __future__ import annotations

from datetime import UTC, datetime, timedelta

import pytest

from backtesting.nautilus_backtest import BacktestRunResult, run_backtest

nautilus_trader = pytest.importorskip("nautilus_trader")


def _synthetic_bars(count: int = 200, *, start_price: float = 100.0) -> list[dict]:
    """Deterministic up-then-down sawtooth — guarantees at least one EMA crossover each way."""
    base = datetime(2024, 1, 1, tzinfo=UTC)
    bars = []
    half = count // 2
    price = start_price
    for i in range(count):
        price += 1.0 if i < half else -1.0
        ts = base + timedelta(minutes=i)
        bars.append(
            {
                "ts": ts,
                "open": price - 0.5,
                "high": price + 0.5,
                "low": price - 0.5,
                "close": price,
                "volume": 10.0,
            }
        )
    return bars


def test_run_backtest_unknown_strategy_raises_key_error() -> None:
    with pytest.raises(KeyError):
        run_backtest(symbol="BTC-USD", strategy_key="does_not_exist", bars=_synthetic_bars())


def test_run_backtest_empty_bars_raises_value_error() -> None:
    with pytest.raises(ValueError, match="non-empty"):
        run_backtest(symbol="BTC-USD", strategy_key="ema_cross", bars=[])


def test_run_backtest_ema_cross_produces_structured_result() -> None:
    result = run_backtest(
        symbol="BTC-USD",
        strategy_key="ema_cross",
        bars=_synthetic_bars(),
        strategy_params={"fast_ema_period": 5, "slow_ema_period": 10, "trade_size": "0.01"},
    )

    assert isinstance(result, BacktestRunResult)
    assert result.symbol == "BTC-USD"
    assert result.strategy_key == "ema_cross"
    assert result.bar_count == 200
    assert result.start is not None and result.end is not None
    assert result.iterations > 0
    assert result.total_orders > 0
    assert "USD" in result.stats_pnls
    assert isinstance(result.stats_pnls["USD"].get("PnL (total)"), float)
    assert len(result.fills) == result.total_orders
    assert all(isinstance(f, dict) for f in result.fills)
    assert all(isinstance(p, dict) for p in result.positions)


def test_run_backtest_uses_strategy_default_params_when_unset() -> None:
    result = run_backtest(symbol="ETH-USD", strategy_key="ema_cross", bars=_synthetic_bars(120))
    assert result.strategy_params["fast_ema_period"] == 10
    assert result.strategy_params["slow_ema_period"] == 20
    assert result.strategy_params["trade_size"] == "0.01"


def test_to_dict_is_json_serializable() -> None:
    import json

    result = run_backtest(symbol="BTC-USD", strategy_key="ema_cross", bars=_synthetic_bars())
    json.dumps(result.to_dict())
