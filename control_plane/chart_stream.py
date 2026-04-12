"""Server-Sent Events stream for chart bar updates (FB-AP-034)."""

from __future__ import annotations

import asyncio
import json
import time
from datetime import UTC, datetime
from typing import Any, AsyncIterator

from app.config.settings import AppSettings
from control_plane.chart_bars import _row_to_json
from data_plane.storage.questdb import QuestDBWriter


def _normalize_ts(ts: Any) -> datetime | None:
    if ts is None:
        return None
    if isinstance(ts, datetime):
        return ts if ts.tzinfo else ts.replace(tzinfo=UTC)
    return None


async def sse_chart_bar_updates(
    settings: AppSettings,
    *,
    symbol: str,
    interval_seconds: int | None,
    poll_seconds: float,
) -> AsyncIterator[str]:
    """
    ``text/event-stream`` lines: JSON events with latest canonical bar when ``ts`` advances.

    Emits ``: ping`` comments periodically for proxies; closes on client disconnect (generator stop).
    """
    sym = symbol.strip()
    if not sym:
        yield f"data: {json.dumps({'type': 'error', 'message': 'symbol is required'})}\n\n"
        return

    bar_sec = (
        int(interval_seconds)
        if interval_seconds is not None
        else max(1, int(settings.market_data_bar_interval_seconds))
    )
    if bar_sec < 1:
        yield f"data: {json.dumps({'type': 'error', 'message': 'interval_seconds must be >= 1'})}\n\n"
        return

    poll = max(0.5, min(float(poll_seconds), 60.0))

    qdb = QuestDBWriter(
        settings.questdb_host,
        settings.questdb_port,
        settings.questdb_user,
        settings.questdb_password,
        settings.questdb_database,
        batch_max_rows=settings.questdb_batch_max_rows,
    )
    try:
        await qdb.connect()
    except Exception as e:
        yield f"data: {json.dumps({'type': 'error', 'message': f'questdb: {e}'})}\n\n"
        return

    last_emitted_ts: datetime | None = None
    last_ping = time.monotonic()

    try:
        hello = {
            "type": "hello",
            "symbol": sym,
            "interval_seconds": bar_sec,
            "poll_seconds": poll,
        }
        yield f"data: {json.dumps(hello)}\n\n"

        while True:
            try:
                row = await qdb.query_latest_bar(sym, interval_seconds=bar_sec)
            except Exception as e:
                yield f"data: {json.dumps({'type': 'error', 'message': str(e)})}\n\n"
                await asyncio.sleep(poll)
                continue

            if row:
                ts = _normalize_ts(row.get("ts"))
                if ts is not None and ts != last_emitted_ts:
                    last_emitted_ts = ts
                    payload = {
                        "type": "bar",
                        "bar": _row_to_json(row),
                    }
                    yield f"data: {json.dumps(payload, default=str)}\n\n"

            await asyncio.sleep(poll)
            now = time.monotonic()
            if now - last_ping >= 15.0:
                yield ": ping\n\n"
                last_ping = now
    finally:
        await qdb.aclose()
