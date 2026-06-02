"""In-memory OHLCV roll-up from tick/stream prices (live path). Bucket size configurable in seconds (default 1s)."""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

import polars as pl


class RollingBars:
    """Builds fixed-interval buckets from ticks; exposes Polars frame for FeaturePipeline."""

    def __init__(self, symbol: str, *, interval_seconds: int = 1, max_completed: int = 512) -> None:
        if interval_seconds < 1:
            raise ValueError("interval_seconds must be >= 1")
        self.symbol = symbol
        self.interval_seconds = interval_seconds
        self.max_completed = max_completed
        self._completed: list[dict] = []
        self._bucket_start: datetime | None = None
        self._o = self._h = self._l = self._c = 0.0
        self._v = 0.0

    def _bucket_floor(self, ts: datetime) -> datetime:
        t = ts if ts.tzinfo else ts.replace(tzinfo=UTC)
        t = t.astimezone(UTC)
        epoch = int(t.timestamp())
        floored = epoch - (epoch % self.interval_seconds)
        return datetime.fromtimestamp(floored, tz=UTC)

    def seed(self, rows: "list[dict] | pl.DataFrame") -> None:
        """Pre-populate completed buckets from historical OHLCV rows.

        Rows must contain columns: ``timestamp``, ``open``, ``high``, ``low``, ``close``,
        ``volume``. They are sorted ascending, trimmed to ``max_completed``, and loaded into
        ``_completed``. ``_bucket_start`` is advanced to the last seeded row's bucket floor so
        the first ``on_tick`` after seeding continues contiguously (Phase C warm-start).
        """
        if isinstance(rows, pl.DataFrame):
            records: list[dict] = rows.sort("timestamp").to_dicts()
        else:
            records = sorted(rows, key=lambda r: r.get("timestamp") or 0)
        if not records:
            return
        # Keep only the tail that fits in max_completed.
        records = records[-self.max_completed :]
        self._completed = [
            {
                "timestamp": r["timestamp"],
                "open": float(r["open"]),
                "high": float(r["high"]),
                "low": float(r["low"]),
                "close": float(r["close"]),
                "volume": float(r["volume"]),
            }
            for r in records
        ]
        # Reset to "no partial in progress" — the first live on_tick will open a new bucket.
        # This prevents the first tick from spuriously closing a synthetic partial seeded from
        # the last historical bar.
        self._bucket_start = None
        self._o = self._h = self._l = self._c = 0.0
        self._v = 0.0

    def bars_frame_completed(self) -> pl.DataFrame:
        """Completed bars only (no in-progress bucket) — used for deterministic forecaster input."""
        if not self._completed:
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
        return pl.DataFrame(self._completed).sort("timestamp")

    def on_tick(self, price: float, ts: datetime, size: float = 0.0) -> dict[str, Any] | None:
        """Update current bucket or roll to a new bar.

        When the bucket **closes** (advance to a new interval), returns the **completed** OHLCV row
        (``timestamp`` = bucket start UTC, same shape as bootstrap / canonical Parquet). The
        in-progress bucket is **not** returned until it closes (**FB-AP-016**).
        """
        b = self._bucket_floor(ts)
        if self._bucket_start is None:
            self._bucket_start = b
            self._o = self._h = self._l = self._c = float(price)
            self._v = float(size)
            return None
        if b > self._bucket_start:
            completed = {
                "timestamp": self._bucket_start,
                "open": self._o,
                "high": self._h,
                "low": self._l,
                "close": self._c,
                "volume": self._v,
            }
            self._completed.append(completed)
            if len(self._completed) > self.max_completed:
                self._completed.pop(0)
            self._bucket_start = b
            self._o = self._h = self._l = self._c = float(price)
            self._v = float(size)
            return completed
        p = float(price)
        self._h = max(self._h, p)
        self._l = min(self._l, p)
        self._c = p
        self._v += float(size)
        return None

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
        """Completed bars + in-progress bucket (for feature enrichment)."""
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


class RollingMinuteBars(RollingBars):
    """Backward-compatible 60-second buckets."""

    def __init__(self, symbol: str, max_completed: int = 512) -> None:
        super().__init__(symbol, interval_seconds=60, max_completed=max_completed)
