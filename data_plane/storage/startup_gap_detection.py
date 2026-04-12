"""Startup gap detection for canonical bars (FB-AP-018).

For each **initialized** symbol (manifest present), compare QuestDB **max(ts)** for the live
``interval_seconds`` to the **last closed** bucket start at wall-clock UTC. If storage is behind,
report a gap range for operators and **FB-AP-019** backfill (not implemented here).
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime, timedelta

from data_plane.storage.questdb import QuestDBWriter


def last_closed_bucket_start_utc(now: datetime, interval_seconds: int) -> datetime:
    """UTC **start** of the most recent **fully closed** bar of width ``interval_seconds``."""
    if interval_seconds < 1:
        raise ValueError("interval_seconds must be >= 1")
    t = now.astimezone(UTC)
    e = int(t.timestamp())
    floored = e - (e % interval_seconds)
    closed_start = floored - interval_seconds
    return datetime.fromtimestamp(closed_start, tz=UTC)


@dataclass(frozen=True)
class CanonicalBarGap:
    """One symbol: stored head vs expected last closed bucket."""

    symbol: str
    interval_seconds: int
    max_stored_ts: datetime | None
    wall_clock_utc: datetime
    last_closed_bucket_start: datetime
    gap_detected: bool
    gap_start: datetime | None
    """First missing bucket start (UTC); ``None`` if there is no row in QuestDB for this key."""
    gap_end: datetime
    """Last bucket start we expect through (inclusive): ``last_closed_bucket_start``."""
    behind_seconds: float | None
    """Seconds between ``max_stored_ts`` and ``last_closed_bucket_start`` when both exist; ``None`` if no rows."""

    def to_log_dict(self) -> dict[str, object]:
        return {
            "symbol": self.symbol,
            "interval_seconds": self.interval_seconds,
            "max_stored_ts": self.max_stored_ts.isoformat() if self.max_stored_ts else None,
            "last_closed_bucket_start": self.last_closed_bucket_start.isoformat(),
            "gap_detected": self.gap_detected,
            "gap_start": self.gap_start.isoformat() if self.gap_start else None,
            "gap_end": self.gap_end.isoformat(),
            "behind_seconds": self.behind_seconds,
        }


async def detect_canonical_bar_gaps(
    qdb: QuestDBWriter,
    *,
    symbols: list[str],
    interval_seconds: int,
    wall_clock_utc: datetime | None = None,
) -> list[CanonicalBarGap]:
    """
    For each symbol, load max ``ts`` from ``canonical_bars`` for ``interval_seconds``.

    A **gap** exists when ``max_stored`` is ``None`` or strictly before ``last_closed_bucket_start``.
    """
    now = wall_clock_utc.astimezone(UTC) if wall_clock_utc else datetime.now(UTC)
    last_closed = last_closed_bucket_start_utc(now, interval_seconds)
    out: list[CanonicalBarGap] = []
    for sym in symbols:
        mx = await qdb.max_canonical_bar_timestamp(sym, interval_seconds=interval_seconds)
        if mx is not None and mx.tzinfo is None:
            mx = mx.replace(tzinfo=UTC)
        elif mx is not None:
            mx = mx.astimezone(UTC)

        if mx is None:
            gap_detected = True
            gap_start = None
            behind: float | None = None
        else:
            gap_detected = mx < last_closed
            gap_start = (
                mx + timedelta(seconds=interval_seconds) if gap_detected else None
            )
            behind = (
                max(0.0, (last_closed - mx).total_seconds()) if gap_detected else 0.0
            )

        out.append(
            CanonicalBarGap(
                symbol=sym,
                interval_seconds=interval_seconds,
                max_stored_ts=mx,
                wall_clock_utc=now,
                last_closed_bucket_start=last_closed,
                gap_detected=gap_detected,
                gap_start=gap_start,
                gap_end=last_closed,
                behind_seconds=behind,
            )
        )
    return out
