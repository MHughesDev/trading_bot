"""In-memory 1-minute OHLCV roll-up from tick/stream prices (live path)."""

from __future__ import annotations

from datetime import UTC, datetime

import polars as pl


class RollingMinuteBars:
    """Builds minute buckets from ticks; exposes Polars frame for FeaturePipeline."""

    def __init__(self, symbol: str, max_completed: int = 512) -> None:
        self.symbol = symbol
        self.max_completed = max_completed
        self._completed: list[dict] = []
        self._bucket_start: datetime | None = None
        self._o = self._h = self._l = self._c = 0.0
        self._v = 0.0

    @staticmethod
    def _minute_floor(ts: datetime) -> datetime:
        t = ts if ts.tzinfo else ts.replace(tzinfo=UTC)
        t = t.astimezone(UTC)
        return t.replace(second=0, microsecond=0)

    def on_tick(self, price: float, ts: datetime, size: float = 0.0) -> None:
        """Update current minute or roll to a new bar."""
        b = self._minute_floor(ts)
        if self._bucket_start is None:
            self._bucket_start = b
            self._o = self._h = self._l = self._c = float(price)
            self._v = float(size)
            return
        if b > self._bucket_start:
            self._completed.append(
                {
                    "timestamp": self._bucket_start,
                    "open": self._o,
                    "high": self._h,
                    "low": self._l,
                    "close": self._c,
                    "volume": self._v,
                }
            )
            if len(self._completed) > self.max_completed:
                self._completed.pop(0)
            self._bucket_start = b
            self._o = self._h = self._l = self._c = float(price)
            self._v = float(size)
        else:
            p = float(price)
            self._h = max(self._h, p)
            self._l = min(self._l, p)
            self._c = p
            self._v += float(size)

    def current_partial_row(self) -> dict | None:
        if self._bucket_start is None:
            return None
        return {
            "timestamp": self._bucket_start,
            "open": self._o,
            "high": self._h,
            "low": self._l,
            "close": self._c,
            "volume": self._v,
        }

    def bars_frame_with_partial(self) -> pl.DataFrame:
        """Completed bars + in-progress minute (for feature enrichment)."""
        rows = list(self._completed)
        partial = self.current_partial_row()
        if partial:
            rows = rows + [partial]
        if not rows:
            return pl.DataFrame(
                schema={
                    "timestamp": pl.Datetime(time_zone="UTC"),
                    "open": pl.Float64,
                    "high": pl.Float64,
                    "low": pl.Float64,
                    "close": pl.Float64,
                    "volume": pl.Float64,
                }
            )
        return pl.DataFrame(rows).sort("timestamp")
