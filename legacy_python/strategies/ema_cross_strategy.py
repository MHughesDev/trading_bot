"""Example strategy: a simple dual-EMA crossover, in NautilusTrader's native shape.

This is the reference strategy for the platform's backtest engine (see
``backtesting/nautilus_backtest.py``) and the pattern every other strategy in this folder
should follow: a frozen :class:`StrategyConfig` describing its tunable parameters, and a
:class:`Strategy` subclass implementing the event handlers (``on_start``/``on_bar``/``on_stop``).

Goes long when the fast EMA crosses above the slow EMA, short on the reverse cross, and is
flat otherwise. No alpha claim — it exists to exercise the engine end-to-end and as a
template for hand-written strategies.
"""

from __future__ import annotations

from decimal import Decimal

from nautilus_trader.config import PositiveInt, StrategyConfig
from nautilus_trader.indicators import ExponentialMovingAverage
from nautilus_trader.model.data import Bar, BarType
from nautilus_trader.model.enums import OrderSide, TimeInForce
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.trading.strategy import Strategy


class EMACrossStrategyConfig(StrategyConfig, frozen=True):
    """Tunable parameters for :class:`EMACrossStrategy`.

    Parameters
    ----------
    instrument_id : InstrumentId
        The instrument to trade.
    bar_type : BarType
        The bar type to subscribe to and act on.
    trade_size : Decimal
        Position size (in base-asset units) per entry.
    fast_ema_period : int, default 10
        Period of the fast EMA.
    slow_ema_period : int, default 20
        Period of the slow EMA (must be greater than ``fast_ema_period``).
    """

    instrument_id: InstrumentId
    bar_type: BarType
    trade_size: Decimal
    fast_ema_period: PositiveInt = 10
    slow_ema_period: PositiveInt = 20


class EMACrossStrategy(Strategy):
    """Dual-EMA crossover: long on a bullish cross, short on a bearish cross, else flat."""

    def __init__(self, config: EMACrossStrategyConfig) -> None:
        if config.fast_ema_period >= config.slow_ema_period:
            raise ValueError("fast_ema_period must be less than slow_ema_period")
        super().__init__(config)
        self.instrument: Instrument | None = None
        self.fast_ema = ExponentialMovingAverage(config.fast_ema_period)
        self.slow_ema = ExponentialMovingAverage(config.slow_ema_period)

    def on_start(self) -> None:
        self.instrument = self.cache.instrument(self.config.instrument_id)
        if self.instrument is None:
            self.log.error(f"Could not find instrument for {self.config.instrument_id}")
            self.stop()
            return
        self.register_indicator_for_bars(self.config.bar_type, self.fast_ema)
        self.register_indicator_for_bars(self.config.bar_type, self.slow_ema)
        self.subscribe_bars(self.config.bar_type)

    def on_bar(self, bar: Bar) -> None:
        if not self.indicators_initialized():
            return
        if bar.is_single_price():
            return

        instrument_id = self.config.instrument_id
        qty = self.instrument.make_qty(self.config.trade_size)

        if self.fast_ema.value > self.slow_ema.value:
            if self.portfolio.is_net_short(instrument_id):
                self.close_all_positions(instrument_id)
            if self.portfolio.is_flat(instrument_id):
                self._submit_market(OrderSide.BUY, qty)
        elif self.fast_ema.value < self.slow_ema.value:
            if self.portfolio.is_net_long(instrument_id):
                self.close_all_positions(instrument_id)
            if self.portfolio.is_flat(instrument_id):
                self._submit_market(OrderSide.SELL, qty)

    def _submit_market(self, side: OrderSide, quantity) -> None:
        order = self.order_factory.market(
            instrument_id=self.config.instrument_id,
            order_side=side,
            quantity=quantity,
            time_in_force=TimeInForce.GTC,
        )
        self.submit_order(order)

    def on_stop(self) -> None:
        self.cancel_all_orders(self.config.instrument_id)
        self.close_all_positions(self.config.instrument_id)
        self.unsubscribe_bars(self.config.bar_type)

    def on_reset(self) -> None:
        self.fast_ema.reset()
        self.slow_ema.reset()
