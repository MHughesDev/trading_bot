"""Generic rule-interpreting strategy: runs any :mod:`strategies.rule_spec` JSON spec.

This is the runtime half of the strategy builder (FB-AP-XXX): every strategy a user assembles
in the UI — sentence/block builder, node graph, or code mode — compiles down to a
:class:`strategies.rule_spec.RuleStrategySpec`, which this single :class:`Strategy` interprets
at runtime. One generic class means the rest of the platform (``backtesting/nautilus_backtest.py``,
``app.runtime.asset_strategy_selection``, the registry) needs **zero** changes to support an
unbounded number of user-defined strategies — each is just another ``StrategyDescriptor`` whose
``rule_spec`` parameter carries a different JSON string (see
:func:`strategies.custom_strategy_store.register_custom_strategies`).

Indicators, conditions, sizing, and exits are all driven off the spec; see
``strategies/rule_spec.py`` for the schema and ``strategies/ema_cross_strategy.py`` for the
canonical hand-written reference this mirrors.
"""

from __future__ import annotations

from decimal import Decimal
from typing import TYPE_CHECKING

from nautilus_trader.config import StrategyConfig
from nautilus_trader.indicators import (
    AverageTrueRange,
    ExponentialMovingAverage,
    RelativeStrengthIndex,
    SimpleMovingAverage,
)
from nautilus_trader.model.data import Bar, BarType
from nautilus_trader.model.enums import OrderSide, TimeInForce
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.trading.strategy import Strategy

from strategies.rule_spec import Condition, IndicatorSpec, RuleSpecError, RuleStrategySpec

if TYPE_CHECKING:
    from nautilus_trader.indicators.base.indicator import Indicator


_INDICATOR_FACTORIES = {
    "ema": ExponentialMovingAverage,
    "sma": SimpleMovingAverage,
    "rsi": RelativeStrengthIndex,
    "atr": AverageTrueRange,
}


class RuleBasedStrategyConfig(StrategyConfig, frozen=True):
    """Tunable parameters for :class:`RuleBasedStrategy`.

    Parameters
    ----------
    instrument_id : InstrumentId
        The instrument to trade.
    bar_type : BarType
        The bar type to subscribe to and act on.
    rule_spec : str
        JSON-encoded :class:`strategies.rule_spec.RuleStrategySpec` (a string, not a nested
        object — Nautilus configs must be built from serialisable primitives).
    """

    instrument_id: InstrumentId
    bar_type: BarType
    rule_spec: str


class RuleBasedStrategy(Strategy):
    """Interprets a :class:`strategies.rule_spec.RuleStrategySpec` against live/replayed bars.

    Builds one Nautilus indicator per :class:`~strategies.rule_spec.IndicatorSpec`, evaluates
    the entry rule's conditions on each completed bar, and manages a single net position with
    fixed-fraction stop-loss / take-profit exits described by the spec.
    """

    def __init__(self, config: RuleBasedStrategyConfig) -> None:
        spec = RuleStrategySpec.from_json(config.rule_spec)
        spec.validate()
        super().__init__(config)
        self.spec = spec
        self.instrument: Instrument | None = None
        self._indicators: dict[str, Indicator] = {
            ind.id: _build_indicator(ind) for ind in spec.indicators
        }
        self._prev_values: dict[str, float] = {}
        self._entry_price: Decimal | None = None

    def on_start(self) -> None:
        self.instrument = self.cache.instrument(self.config.instrument_id)
        if self.instrument is None:
            self.log.error(f"Could not find instrument for {self.config.instrument_id}")
            self.stop()
            return
        for indicator in self._indicators.values():
            self.register_indicator_for_bars(self.config.bar_type, indicator)
        self.subscribe_bars(self.config.bar_type)

    def on_bar(self, bar: Bar) -> None:
        if not self.indicators_initialized():
            return
        if bar.is_single_price():
            return

        instrument_id = self.config.instrument_id
        price = float(bar.close)

        if not self.portfolio.is_flat(instrument_id):
            if self._exit_triggered(price):
                self.close_all_positions(instrument_id)
                self._entry_price = None
            self._record_indicator_values(price)
            return

        if self._entry_triggered(price):
            self._enter(price)

        self._record_indicator_values(price)

    def _entry_triggered(self, price: float) -> bool:
        entry = self.spec.entry
        if entry is None:
            return False
        if entry.all_of and not all(self._evaluate(c, price) for c in entry.all_of):
            return False
        if entry.any_of and not any(self._evaluate(c, price) for c in entry.any_of):
            return False
        return bool(entry.all_of or entry.any_of)

    def _evaluate(self, condition: Condition, price: float) -> bool:
        left = self._value_of(condition.left, price)
        if left is None:
            return False
        prev_left = self._prev_values.get(condition.left)

        if condition.type == "rising":
            return prev_left is not None and left > prev_left
        if condition.type == "falling":
            return prev_left is not None and left < prev_left

        right_key = condition.right_id if condition.right_id is not None else None
        right = (
            self._value_of(right_key, price)
            if right_key is not None
            else condition.right_value
        )
        if right is None:
            return False

        if condition.type == "greater_than":
            return left > right
        if condition.type == "less_than":
            return left < right
        if condition.type in ("cross_above", "cross_below"):
            prev_right = (
                self._prev_values.get(right_key) if right_key is not None else condition.right_value
            )
            if prev_left is None or prev_right is None:
                return False
            if condition.type == "cross_above":
                return prev_left <= prev_right and left > right
            return prev_left >= prev_right and left < right
        return False

    def _value_of(self, ref: str | None, price: float) -> float | None:
        if ref is None:
            return None
        if ref == "price":
            return price
        indicator = self._indicators.get(ref)
        if indicator is None or not indicator.initialized:
            return None
        return float(indicator.value)

    def _record_indicator_values(self, price: float) -> None:
        self._prev_values["price"] = price
        for indicator_id, indicator in self._indicators.items():
            if indicator.initialized:
                self._prev_values[indicator_id] = float(indicator.value)

    def _enter(self, price: float) -> None:
        instrument_id = self.config.instrument_id
        size = self.spec.size
        if size.type == "percent_of_equity":
            account = self.portfolio.account(instrument_id.venue)
            if account is None or price <= 0:
                return
            balance = account.balance_total(self.instrument.quote_currency)
            equity = float(balance) if balance is not None else 0.0
            notional = equity * size.value
            if notional <= 0:
                return
            quantity = self.instrument.make_qty(Decimal(str(notional / price)))
        else:
            quantity = self.instrument.make_qty(Decimal(str(size.value)))
        if quantity is None or quantity.as_double() <= 0:
            return

        side = OrderSide.BUY if self.spec.entry.side == "buy" else OrderSide.SELL
        order = self.order_factory.market(
            instrument_id=instrument_id,
            order_side=side,
            quantity=quantity,
            time_in_force=TimeInForce.GTC,
        )
        self.submit_order(order)
        self._entry_price = Decimal(str(price))

    def _exit_triggered(self, price: float) -> bool:
        if self._entry_price is None or self._entry_price == 0:
            return False
        entry = float(self._entry_price)
        is_long = self.portfolio.is_net_long(self.config.instrument_id)
        move = (price - entry) / entry if is_long else (entry - price) / entry

        for rule in self.spec.exits:
            if rule.type == "stop_loss" and move <= -rule.value:
                return True
            if rule.type in ("take_profit", "trailing_stop") and move >= rule.value:
                return True
        return False

    def on_stop(self) -> None:
        self.cancel_all_orders(self.config.instrument_id)
        self.close_all_positions(self.config.instrument_id)
        self.unsubscribe_bars(self.config.bar_type)
        self._entry_price = None

    def on_reset(self) -> None:
        for indicator in self._indicators.values():
            indicator.reset()
        self._prev_values.clear()
        self._entry_price = None


def _build_indicator(spec: IndicatorSpec) -> "Indicator":
    factory = _INDICATOR_FACTORIES.get(spec.kind)
    if factory is None:
        raise RuleSpecError(f"unsupported indicator kind for execution: {spec.kind!r}")
    return factory(spec.period)
