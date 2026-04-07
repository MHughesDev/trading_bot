from __future__ import annotations

from dataclasses import dataclass, field
from datetime import UTC, datetime

from app.contracts.events import BarEvent, TradeEvent


def _bucket_start(ts: datetime, interval_seconds: int) -> datetime:
    ts = ts.astimezone(UTC)
    epoch = int(ts.timestamp())
    bucket = epoch - (epoch % interval_seconds)
    return datetime.fromtimestamp(bucket, tz=UTC)


@dataclass(slots=True)
class OhlcvAccumulator:
    symbol: str
    start: datetime
    open: float
    high: float
    low: float
    close: float
    volume: float = 0.0

    def add_trade(self, trade: TradeEvent) -> None:
        price = trade.price
        self.high = max(self.high, price)
        self.low = min(self.low, price)
        self.close = price
        self.volume += trade.size

    def to_bar(self) -> BarEvent:
        return BarEvent(
            timestamp=self.start,
            symbol=self.symbol,
            open=self.open,
            high=self.high,
            low=self.low,
            close=self.close,
            volume=self.volume,
        )


@dataclass(slots=True)
class TradeBarAggregator:
    interval_seconds: int = 60
    _accumulators: dict[str, OhlcvAccumulator] = field(default_factory=dict)

    def update(self, trade: TradeEvent) -> BarEvent | None:
        start = _bucket_start(trade.timestamp, self.interval_seconds)
        acc = self._accumulators.get(trade.symbol)
        if acc is None:
            self._accumulators[trade.symbol] = OhlcvAccumulator(
                symbol=trade.symbol,
                start=start,
                open=trade.price,
                high=trade.price,
                low=trade.price,
                close=trade.price,
                volume=trade.size,
            )
            return None

        if acc.start == start:
            acc.add_trade(trade)
            return None

        # Bucket rolled.
        finished = acc.to_bar()
        self._accumulators[trade.symbol] = OhlcvAccumulator(
            symbol=trade.symbol,
            start=start,
            open=trade.price,
            high=trade.price,
            low=trade.price,
            close=trade.price,
            volume=trade.size,
        )
        return finished

    def flush(self) -> list[BarEvent]:
        bars: list[BarEvent] = []
        for symbol in list(self._accumulators.keys()):
            bars.append(self._accumulators[symbol].to_bar())
            del self._accumulators[symbol]
        return bars
