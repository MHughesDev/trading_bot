"""Symbol-scoped canonical OHLCV read for charts (FB-AP-024)."""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

from app.config.settings import AppSettings
from data_plane.storage.questdb import QuestDBWriter


def _normalize_utc(dt: datetime) -> datetime:
    if dt.tzinfo is None:
        return dt.replace(tzinfo=UTC)
    return dt.astimezone(UTC)


def _row_to_json(row: dict[str, Any]) -> dict[str, Any]:
    ts = row["ts"]
    if isinstance(ts, datetime):
        t = ts if ts.tzinfo else ts.replace(tzinfo=UTC)
        ts_out = t.astimezone(UTC).isoformat()
    else:
        ts_out = str(ts)
    return {
        "ts": ts_out,
        "symbol": row["symbol"],
        "interval_seconds": int(row["interval_seconds"]),
        "open": float(row["open"]),
        "high": float(row["high"]),
        "low": float(row["low"]),
        "close": float(row["close"]),
        "volume": float(row["volume"]),
        "source": row.get("source"),
        "schema_version": int(row["schema_version"]) if row.get("schema_version") is not None else None,
    }


async def query_canonical_bars_for_chart(
    settings: AppSettings,
    *,
    symbol: str,
    start: datetime,
    end: datetime,
    interval_seconds: int | None,
    limit: int,
) -> dict[str, Any]:
    """
    Read ``canonical_bars`` for a single symbol and time window.

    Raises:
        ValueError: empty symbol, or ``start >= end``.
    """
    sym = symbol.strip()
    if not sym:
        raise ValueError("symbol is required")
    s = _normalize_utc(start)
    e = _normalize_utc(end)
    if s >= e:
        raise ValueError("start must be before end")
    bar_sec = (
        int(interval_seconds)
        if interval_seconds is not None
        else max(1, int(settings.market_data_bar_interval_seconds))
    )
    if bar_sec < 1:
        raise ValueError("interval_seconds must be >= 1")
    lim = max(1, min(int(limit), 50_000))
    qdb = QuestDBWriter(
        settings.questdb_host,
        settings.questdb_port,
        settings.questdb_user,
        settings.questdb_password,
        settings.questdb_database,
        batch_max_rows=settings.questdb_batch_max_rows,
    )
    await qdb.connect()
    try:
        rows = await qdb.query_bars(sym, s, e, limit=lim, interval_seconds=bar_sec)
    finally:
        await qdb.aclose()
    return {
        "symbol": sym,
        "interval_seconds": bar_sec,
        "start": s.isoformat(),
        "end": e.isoformat(),
        "limit": lim,
        "count": len(rows),
        "bars": [_row_to_json(r) for r in rows],
    }
