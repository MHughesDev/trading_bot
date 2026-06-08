"""End-to-end test: a builder-created strategy runs through the existing backtest pipeline
unchanged (FB-AP-XXX) — the integration claim behind the strategy builder.

Requires the optional `nautilus_trader` extra, mirroring `test_nautilus_backtest.py`.
"""

from __future__ import annotations

from datetime import UTC, datetime, timedelta

import pytest

from strategies.custom_strategy_store import save_custom_strategy
from strategies.rule_spec import Condition, EntryRule, ExitRule, IndicatorSpec, RuleStrategySpec, SizeRule

nautilus_trader = pytest.importorskip("nautilus_trader")

from backtesting.nautilus_backtest import run_backtest  # noqa: E402


def _trending_bars(count: int = 200, *, start_price: float = 100.0) -> list[dict]:
    base = datetime(2024, 1, 1, tzinfo=UTC)
    bars = []
    price = start_price
    for i in range(count):
        price += 1.0
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


_USER_ID = 1


@pytest.fixture
def custom_strategies_db(tmp_path):
    return tmp_path / "users.sqlite"


def _save_ema_strategy(db_path) -> str:
    from strategies.custom_strategy_store import registry_key

    spec = RuleStrategySpec(
        name="Builder EMA strategy",
        indicators=(
            IndicatorSpec(id="ema_fast", kind="ema", period=7),
            IndicatorSpec(id="ema_slow", kind="ema", period=21),
        ),
        entry=EntryRule(
            side="buy",
            all_of=(Condition(type="greater_than", left="ema_fast", right_id="ema_slow"),),
        ),
        size=SizeRule(type="percent_of_equity", value=0.02),
        exits=(ExitRule(type="stop_loss", value=0.015), ExitRule(type="take_profit", value=0.04)),
    )
    record = save_custom_strategy(db_path, _USER_ID, spec)
    return registry_key(_USER_ID, record["id"])


def test_custom_strategy_runs_through_existing_backtest_pipeline(custom_strategies_db) -> None:
    key = _save_ema_strategy(custom_strategies_db)

    result = run_backtest(symbol="BTC-USD", strategy_key=key, bars=_trending_bars())

    assert result.strategy_key == key
    assert result.bar_count == 200
    assert result.total_orders > 0
    assert "rule_spec" in result.strategy_params


def test_custom_strategy_appears_in_catalogue(custom_strategies_db) -> None:
    from strategies.custom_strategy_store import register_custom_strategies
    from strategies.registry import get_strategy, list_strategies

    key = _save_ema_strategy(custom_strategies_db)
    register_custom_strategies(custom_strategies_db)

    assert get_strategy(key) is not None
    assert any(d.key == key for d in list_strategies())
