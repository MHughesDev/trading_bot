"""Per-symbol OHLCV data-health check (Phase D).

Evaluates the seeded ``RollingBars`` history for three properties:

- **deep** — at least ``required_bars`` completed bars present.
- **contiguous** — no interior gap exceeding ``max_gap_seconds`` (catches Kraken 720-cap residuals).
- **recent** — latest bar timestamp is within ``max_staleness_seconds`` of ``now``.

The check feeds the risk engine: unhealthy → ``SystemMode.PAUSE_NEW_ENTRIES`` + reason code
``RISK_BLOCK_DATA_HEALTH`` so no new positions open on poisoned history.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import UTC, datetime, timedelta
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    import polars as pl

# Matches the constant in risk_engine/engine.py — defined here for import-boundary cleanliness.
RISK_BLOCK_DATA_HEALTH = "risk_data_health"


@dataclass(frozen=True)
class DataHealthResult:
    symbol: str
    is_healthy: bool
    bar_count: int
    has_interior_gap: bool
    is_stale: bool
    is_shallow: bool
    reasons: list[str] = field(default_factory=list)
    checked_at: datetime = field(default_factory=lambda: datetime.now(UTC))

    def to_log_dict(self) -> dict:
        return {
            "symbol": self.symbol,
            "is_healthy": self.is_healthy,
            "bar_count": self.bar_count,
            "has_interior_gap": self.has_interior_gap,
            "is_stale": self.is_stale,
            "is_shallow": self.is_shallow,
            "reasons": self.reasons,
            "checked_at": self.checked_at.isoformat(),
        }


def check_data_health(
    symbol: str,
    bars: "pl.DataFrame | None",
    *,
    required_bars: int,
    interval_seconds: int,
    max_staleness_seconds: int = 300,
    gap_tolerance_multiplier: float = 2.0,
    now: datetime | None = None,
) -> DataHealthResult:
    """Check ``bars`` for depth, continuity, and recency.

    Parameters
    ----------
    symbol:
        Symbol name for logging.
    bars:
        Completed OHLCV history (``bars_frame_completed()``). May be None or empty.
    required_bars:
        Minimum number of completed bars needed to consider history *deep*.
    interval_seconds:
        Expected interval between consecutive bar timestamps.
    max_staleness_seconds:
        Maximum seconds between the latest bar and ``now`` before considering data stale.
    gap_tolerance_multiplier:
        A gap is flagged when it exceeds ``interval_seconds * gap_tolerance_multiplier``.
    now:
        Reference clock (UTC); defaults to ``datetime.now(UTC)``.
    """
    now_ts = (now or datetime.now(UTC)).astimezone(UTC)
    reasons: list[str] = []
    bar_count = 0
    has_interior_gap = False
    is_stale = False
    is_shallow = False

    if bars is None or bars.height == 0:
        return DataHealthResult(
            symbol=symbol,
            is_healthy=False,
            bar_count=0,
            has_interior_gap=False,
            is_stale=True,
            is_shallow=True,
            reasons=["no bar history available"],
            checked_at=now_ts,
        )

    bar_count = bars.height

    # Depth check
    if bar_count < required_bars:
        is_shallow = True
        reasons.append(
            f"shallow: {bar_count} bars < required {required_bars}"
        )

    # Sort by timestamp for gap and staleness checks.
    sorted_bars = bars.sort("timestamp")
    timestamps = sorted_bars["timestamp"].to_list()

    # Staleness check
    last_ts = timestamps[-1]
    if hasattr(last_ts, "tzinfo") and last_ts.tzinfo is None:
        last_ts = last_ts.replace(tzinfo=UTC)
    elif hasattr(last_ts, "replace"):
        last_ts = last_ts.astimezone(UTC) if last_ts.tzinfo else last_ts.replace(tzinfo=UTC)
    age_seconds = (now_ts - last_ts).total_seconds()
    if age_seconds > max_staleness_seconds:
        is_stale = True
        reasons.append(
            f"stale: last bar {age_seconds:.0f}s ago > max {max_staleness_seconds}s"
        )

    # Interior gap check (only meaningful with ≥ 2 bars)
    if len(timestamps) >= 2:
        max_allowed = timedelta(seconds=interval_seconds * gap_tolerance_multiplier)
        for prev, curr in zip(timestamps, timestamps[1:]):
            # Normalise to datetime if needed
            if not isinstance(prev, datetime):
                prev = datetime.fromisoformat(str(prev))
            if not isinstance(curr, datetime):
                curr = datetime.fromisoformat(str(curr))
            if prev.tzinfo is None:
                prev = prev.replace(tzinfo=UTC)
            if curr.tzinfo is None:
                curr = curr.replace(tzinfo=UTC)
            gap = curr - prev
            if gap > max_allowed:
                has_interior_gap = True
                reasons.append(
                    f"interior gap of {gap.total_seconds():.0f}s detected "
                    f"(allowed {max_allowed.total_seconds():.0f}s)"
                )
                break  # one gap is enough to flag

    is_healthy = not (is_shallow or is_stale or has_interior_gap)
    return DataHealthResult(
        symbol=symbol,
        is_healthy=is_healthy,
        bar_count=bar_count,
        has_interior_gap=has_interior_gap,
        is_stale=is_stale,
        is_shallow=is_shallow,
        reasons=reasons,
        checked_at=now_ts,
    )
