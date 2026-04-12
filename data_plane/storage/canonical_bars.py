"""
Canonical OHLCV bar contract (FB-AP-014).

**Authoritative series:** one row per ``(symbol, ts_bucket_start_utc, interval_seconds)`` with OHLCV.
Bucket start ``ts`` is **UTC**; ``interval_seconds`` is the bar width (e.g. ``1`` for 1s, ``60`` for 1m).

REST backfill, WS roll-ups, QuestDB, and Parquet exports **must** use the same column names and semantics
so merge/dedupe (**FB-AP-015**) can key on ``(symbol, ts, interval_seconds)``.

See also :class:`app.contracts.events.BarEvent` (runtime/event contract) and
:data:`data_plane.bootstrap_bars.CANONICAL_BAR_COLUMNS` (Polars init bootstrap, extended with interval).
"""

from __future__ import annotations

# Default roll-up for live tick → bar (see ``RollingBars`` / ``NM_MARKET_DATA_BAR_INTERVAL_SECONDS``).
CANONICAL_BAR_INTERVAL_SECONDS_DEFAULT: int = 1

# Parquet / Polars column order for persisted canonical bars (symbol + OHLCV + interval).
CANONICAL_BAR_PARQUET_COLUMNS: tuple[str, ...] = (
    "timestamp",
    "symbol",
    "interval_seconds",
    "open",
    "high",
    "low",
    "close",
    "volume",
)
