"""Control-plane orchestration for strategy backtests (FB-AP-XXX).

Bridges the strategy catalogue (:mod:`strategies.registry`) and the backtest engine
(:func:`backtesting.nautilus_backtest.run_backtest`) to the HTTP API: serialize the strategy
dropdown, and run a backtest for a symbol by pulling its canonical bars
(:func:`control_plane.chart_bars.query_canonical_bars_for_chart`) and feeding them to the engine.

Kept separate from ``api.py`` (mirroring ``chart_bars.py``) so it's unit-testable without a
running FastAPI app, and so ``api.py`` stays a thin routing layer.
"""

from __future__ import annotations

from datetime import datetime
from typing import Any

from app.config.settings import AppSettings
from control_plane.chart_bars import query_canonical_bars_for_chart
from strategies.registry import StrategyDescriptor, get_strategy, list_strategies


def _param_to_json(descriptor_param: Any) -> dict[str, Any]:
    return {
        "name": descriptor_param.name,
        "kind": descriptor_param.kind,
        "default": descriptor_param.default,
        "description": descriptor_param.description,
        "minimum": descriptor_param.minimum,
        "maximum": descriptor_param.maximum,
    }


def _descriptor_to_json(descriptor: StrategyDescriptor) -> dict[str, Any]:
    return {
        "key": descriptor.key,
        "name": descriptor.name,
        "description": descriptor.description,
        "params": [_param_to_json(p) for p in descriptor.params],
    }


def list_strategies_payload() -> dict[str, Any]:
    """JSON payload for the strategy dropdown — import-free (no ``nautilus_trader`` needed)."""
    strategies = [_descriptor_to_json(d) for d in list_strategies()]
    return {"count": len(strategies), "strategies": strategies}


async def run_symbol_backtest(
    settings: AppSettings,
    *,
    symbol: str,
    strategy_key: str,
    start: datetime,
    end: datetime,
    strategy_params: dict[str, Any] | None = None,
    interval_seconds: int | None = None,
    limit: int = 50_000,
    starting_balance: str = "100000",
    starting_currency: str = "USD",
    price_precision: int = 2,
    size_precision: int = 6,
) -> dict[str, Any]:
    """Fetch ``symbol`` bars over the window and run ``strategy_key`` against them.

    Returns the engine's :meth:`BacktestRunResult.to_dict`, augmented with the resolved
    ``interval_seconds`` and ``bar_count`` already present in that dict.

    Raises:
        ValueError: empty symbol, ``start >= end``, unknown strategy, or no bars in window.
        ImportError: the ``backtest_nautilus`` extra (the platform's fork) isn't installed.
    """
    sym = symbol.strip()
    if not sym:
        raise ValueError("symbol is required")
    if get_strategy(strategy_key) is None:
        raise ValueError(f"unknown strategy: {strategy_key!r}")

    chart = await query_canonical_bars_for_chart(
        settings,
        symbol=sym,
        start=start,
        end=end,
        interval_seconds=interval_seconds,
        limit=limit,
    )
    bars = chart["bars"]
    if not bars:
        raise ValueError(
            f"no canonical bars for {sym} in the requested window — initialize/backfill the asset first"
        )
    resolved_interval = int(chart["interval_seconds"])

    # Imported here (not at module top) so listing strategies / importing this module never
    # requires the optional nautilus_trader fork — only an actual run does.
    from backtesting.nautilus_backtest import run_backtest

    result = run_backtest(
        symbol=sym,
        strategy_key=strategy_key,
        bars=bars,
        strategy_params=strategy_params,
        starting_balance=starting_balance,
        starting_currency=starting_currency,
        interval_seconds=resolved_interval,
        price_precision=price_precision,
        size_precision=size_precision,
    )
    return result.to_dict()
