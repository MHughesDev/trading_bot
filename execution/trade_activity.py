"""Trade activity signals from append-only markers (FB-AP-037)."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from pathlib import Path

from execution.trade_markers import iter_markers


def symbol_had_trade_in_lookback(
    symbol: str,
    lookback_days: int,
    *,
    now: datetime | None = None,
    markers_file: Path | None = None,
) -> bool:
    """
    True if ``data/trade_markers.jsonl`` has at least one row for ``symbol`` in
    ``[now - lookback_days, now]`` (UTC).

    Used for nightly RL gating: skip heuristic RL when no intent-submit activity in the window.
    """
    end = now or datetime.now(tz=UTC)
    if end.tzinfo is None:
        end = end.replace(tzinfo=UTC)
    end = end.astimezone(UTC)
    start = end - timedelta(days=max(1, int(lookback_days)))
    # ``iter_markers`` uses ``ts < end``; widen end slightly so boundary-second rows count.
    end_exclusive = end + timedelta(seconds=1)
    rows = iter_markers(
        symbol=symbol.strip(), start=start, end=end_exclusive, path=markers_file
    )
    return len(rows) > 0
