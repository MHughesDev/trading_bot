"""Run a registered :mod:`strategies` strategy through NautilusTrader's ``BacktestEngine``.

This is the bridge between the platform's canonical OHLCV bar storage (``canonical_bars`` /
:func:`control_plane.chart_bars.query_canonical_bars_for_chart`) and a hand-written strategy
in :mod:`strategies` — used by the Asset-page "Backtest" panel (FB-AP-XXX) to answer "how would
strategy X have performed on asset Y over this window".

Requires the optional ``backtest_nautilus`` extra (``nautilus_trader`` or the platform's fork —
see ``strategies/README.md``); :func:`run_backtest` raises a clear :class:`ImportError` if it
isn't installed (mirrors :meth:`strategies.registry.StrategyDescriptor.load`).

This module deliberately knows nothing about how bars are fetched — callers pass in canonical
bar mappings (``{"ts", "open", "high", "low", "close", "volume", ...}``, the same shape returned
by ``query_canonical_bars_for_chart``/``QuestDBWriter.query_bars``) so it stays decoupled from
the data plane and is easy to exercise in tests with synthetic data.
"""

from __future__ import annotations

from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from datetime import datetime, UTC
from decimal import Decimal
from typing import Any

from strategies.registry import get_strategy

_VENUE_NAME = "SIM"
_TRADER_ID = "BACKTESTER-001"


@dataclass(frozen=True)
class BacktestRunResult:
    """JSON-serializable outcome of one strategy backtest run."""

    symbol: str
    strategy_key: str
    strategy_params: dict[str, Any]
    bar_count: int
    start: str | None
    end: str | None
    iterations: int
    total_events: int
    total_orders: int
    total_positions: int
    stats_pnls: dict[str, dict[str, float]]
    stats_returns: dict[str, float]
    fills: list[dict[str, Any]]
    positions: list[dict[str, Any]]

    def to_dict(self) -> dict[str, Any]:
        return {
            "symbol": self.symbol,
            "strategy_key": self.strategy_key,
            "strategy_params": self.strategy_params,
            "bar_count": self.bar_count,
            "start": self.start,
            "end": self.end,
            "iterations": self.iterations,
            "total_events": self.total_events,
            "total_orders": self.total_orders,
            "total_positions": self.total_positions,
            "stats_pnls": self.stats_pnls,
            "stats_returns": self.stats_returns,
            "fills": self.fills,
            "positions": self.positions,
        }


def run_backtest(
    *,
    symbol: str,
    strategy_key: str,
    bars: Iterable[Mapping[str, Any]],
    strategy_params: Mapping[str, Any] | None = None,
    starting_balance: Decimal | str | int | float = "100000",
    starting_currency: str = "USD",
    interval_seconds: int = 60,
    price_precision: int = 2,
    size_precision: int = 6,
) -> BacktestRunResult:
    """Run ``strategy_key`` against ``symbol`` over ``bars`` and return structured results.

    Parameters
    ----------
    symbol:
        Canonical ``BASE-QUOTE`` symbol, e.g. ``"BTC-USD"``.
    strategy_key:
        Key of a strategy registered in :mod:`strategies.registry`.
    bars:
        Canonical OHLCV bar mappings, ascending by ``ts``/``timestamp`` — the shape returned by
        ``query_canonical_bars_for_chart``/``QuestDBWriter.query_bars``
        (``ts`` or ``timestamp``, ``open``, ``high``, ``low``, ``close``, ``volume``).
    strategy_params:
        User-supplied overrides merged over the strategy's :meth:`StrategyDescriptor.default_params`.
    starting_balance / starting_currency:
        Simulated account starting balance for the backtest venue.
    interval_seconds:
        Bar width in seconds (must match the cadence of ``bars``); used to build the ``BarType``.
    price_precision / size_precision:
        Decimal precision for the synthetic instrument and its prices/quantities. Bars are
        rounded to ``price_precision``; ``volume`` is rounded to ``size_precision``.

    Raises
    ------
    ImportError:
        If ``nautilus_trader`` (or the platform's fork) isn't installed.
    KeyError:
        If ``strategy_key`` isn't registered.
    ValueError:
        If ``bars`` is empty.
    """
    descriptor = get_strategy(strategy_key)
    if descriptor is None:
        raise KeyError(f"unknown strategy key: {strategy_key!r}")

    bar_rows = list(bars)
    if not bar_rows:
        raise ValueError("bars must be non-empty")

    strategy_cls, config_cls = descriptor.load()

    # Imported lazily — see module docstring; only reached once a strategy actually loads.
    from nautilus_trader.backtest.engine import BacktestEngine, BacktestEngineConfig
    from nautilus_trader.model.data import Bar, BarSpecification, BarType
    from nautilus_trader.model.enums import (
        AccountType,
        AggregationSource,
        BarAggregation,
        OmsType,
        PriceType,
    )
    from nautilus_trader.model.identifiers import InstrumentId, Symbol, Venue
    from nautilus_trader.model.instruments import CurrencyPair
    from nautilus_trader.model.objects import Currency, Money, Price, Quantity

    venue = Venue(_VENUE_NAME)
    raw_symbol = Symbol(symbol)
    instrument_id = InstrumentId(raw_symbol, venue)
    base_code, _, quote_code = symbol.partition("-")

    instrument = CurrencyPair(
        instrument_id=instrument_id,
        raw_symbol=raw_symbol,
        base_currency=Currency.from_str(base_code or symbol),
        quote_currency=Currency.from_str(quote_code or starting_currency),
        price_precision=price_precision,
        size_precision=size_precision,
        price_increment=Price(10**-price_precision, price_precision),
        size_increment=Quantity(10**-size_precision, size_precision),
        ts_event=0,
        ts_init=0,
    )

    bar_spec = _bar_spec_from_interval_seconds(
        int(interval_seconds), BarSpecification=BarSpecification, BarAggregation=BarAggregation, PriceType=PriceType
    )
    bar_type = BarType(instrument_id, bar_spec, AggregationSource.EXTERNAL)
    nautilus_bars = [
        _to_nautilus_bar(row, bar_type=bar_type, price_precision=price_precision, size_precision=size_precision, Bar=Bar, Price=Price, Quantity=Quantity)
        for row in bar_rows
    ]

    engine = BacktestEngine(config=BacktestEngineConfig(trader_id=_TRADER_ID))
    try:
        engine.add_venue(
            venue,
            oms_type=OmsType.NETTING,
            account_type=AccountType.CASH,
            starting_balances=[Money(Decimal(str(starting_balance)), Currency.from_str(starting_currency))],
            base_currency=None,  # multi-currency cash account — required to hold both legs of the pair
        )
        engine.add_instrument(instrument)
        engine.add_data(nautilus_bars)

        params = dict(descriptor.default_params())
        params.update(strategy_params or {})
        params.pop("instrument_id", None)
        params.pop("bar_type", None)
        config = config_cls(instrument_id=instrument_id, bar_type=bar_type, **params)
        engine.add_strategy(strategy_cls(config=config))

        engine.run()
        result = engine.get_result()

        fills = _report_to_records(engine.trader.generate_order_fills_report())
        positions = _report_to_records(engine.trader.generate_positions_report())
    finally:
        engine.dispose()

    return BacktestRunResult(
        symbol=symbol,
        strategy_key=strategy_key,
        strategy_params=params,
        bar_count=len(nautilus_bars),
        start=_ns_to_iso(result.backtest_start),
        end=_ns_to_iso(result.backtest_end),
        iterations=result.iterations,
        total_events=result.total_events,
        total_orders=result.total_orders,
        total_positions=result.total_positions,
        stats_pnls=result.stats_pnls,
        stats_returns=result.stats_returns,
        fills=fills,
        positions=positions,
    )


def _bar_spec_from_interval_seconds(seconds: int, *, BarSpecification: type, BarAggregation: Any, PriceType: Any) -> Any:
    """Pick the coarsest aggregation unit that evenly divides ``seconds`` (Nautilus rejects e.g. 60 SECOND — wants 1 MINUTE)."""
    if seconds <= 0:
        raise ValueError("interval_seconds must be positive")
    if seconds % 86_400 == 0:
        return BarSpecification(seconds // 86_400, BarAggregation.DAY, PriceType.LAST)
    if seconds % 3_600 == 0:
        return BarSpecification(seconds // 3_600, BarAggregation.HOUR, PriceType.LAST)
    if seconds % 60 == 0:
        return BarSpecification(seconds // 60, BarAggregation.MINUTE, PriceType.LAST)
    return BarSpecification(seconds, BarAggregation.SECOND, PriceType.LAST)


def _to_nautilus_bar(
    row: Mapping[str, Any],
    *,
    bar_type: Any,
    price_precision: int,
    size_precision: int,
    Bar: type,
    Price: type,
    Quantity: type,
) -> Any:
    ts = row.get("ts", row.get("timestamp"))
    ts_ns = _to_ns(ts)
    return Bar(
        bar_type=bar_type,
        open=Price(round(float(row["open"]), price_precision), price_precision),
        high=Price(round(float(row["high"]), price_precision), price_precision),
        low=Price(round(float(row["low"]), price_precision), price_precision),
        close=Price(round(float(row["close"]), price_precision), price_precision),
        volume=Quantity(round(float(row["volume"]), size_precision), size_precision),
        ts_event=ts_ns,
        ts_init=ts_ns,
    )


def _to_ns(ts: Any) -> int:
    if isinstance(ts, datetime):
        dt = ts if ts.tzinfo else ts.replace(tzinfo=UTC)
        return int(dt.timestamp() * 1_000_000_000)
    if isinstance(ts, (int, float)):
        return int(ts)
    return int(datetime.fromisoformat(str(ts)).astimezone(UTC).timestamp() * 1_000_000_000)


def _ns_to_iso(ts_ns: int | None) -> str | None:
    if ts_ns is None:
        return None
    return datetime.fromtimestamp(ts_ns / 1_000_000_000, tz=UTC).isoformat()


def _report_to_records(report: Any) -> list[dict[str, Any]]:
    """Flatten a pandas report DataFrame to JSON-safe records (index included)."""
    if report is None or len(report) == 0:
        return []
    records = report.reset_index().to_dict(orient="records")
    return [{k: _json_safe(v) for k, v in record.items()} for record in records]


def _json_safe(value: Any) -> Any:
    if value is None or isinstance(value, (str, int, float, bool)):
        return value
    if isinstance(value, Decimal):
        return str(value)
    return str(value)
