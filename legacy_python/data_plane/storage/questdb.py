"""Append-only writes to QuestDB via PostgreSQL wire protocol."""

from __future__ import annotations

import json
from datetime import UTC, datetime

import psycopg
from psycopg.rows import dict_row

from app.contracts.events import BarEvent
from data_plane.storage.schemas import QUESTDB_CANONICAL_BARS_DDL, ensure_questdb_schema
from observability.metrics import QUESTDB_WRITE_FAIL


class QuestDBWriter:
    def __init__(
        self,
        host: str,
        port: int,
        user: str,
        password: str,
        database: str = "qdb",
        *,
        batch_max_rows: int = 500,
    ) -> None:
        self._conninfo = (
            f"host={host} port={port} user={user} password={password} dbname={database}"
        )
        self._conn: psycopg.AsyncConnection | None = None
        self._batch_max_rows = max(1, int(batch_max_rows))
        self._trace_buffer: list[dict] = []

    async def connect(self) -> None:
        self._conn = await psycopg.AsyncConnection.connect(self._conninfo, autocommit=True)
        # FB-AP chart reads may happen before any writer path: ensure canonical_bars exists.
        await self._conn.execute(QUESTDB_CANONICAL_BARS_DDL)
        await ensure_questdb_schema(self._conn)

    async def aclose(self) -> None:
        if self._conn:
            await self.flush_decision_traces()
            await self._conn.close()
            self._conn = None

    async def insert_bar(self, bar: BarEvent) -> None:
        """
        Idempotent upsert into ``canonical_bars`` (FB-AP-015): **last-write-wins** on
        ``(symbol, ts, interval_seconds)`` — delete existing row then insert.
        """
        if not self._conn:
            raise RuntimeError("not connected")
        try:
            delete_sql = """
            DELETE FROM canonical_bars
            WHERE symbol = %s AND ts = %s AND interval_seconds = %s
            """
            await self._conn.execute(delete_sql, (bar.symbol, bar.timestamp, bar.interval_seconds))
            insert_sql = """
            INSERT INTO canonical_bars (ts, symbol, interval_seconds, open, high, low, close, volume, source, schema_version)
            VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
            """
            await self._conn.execute(
                insert_sql,
                (
                    bar.timestamp,
                    bar.symbol,
                    bar.interval_seconds,
                    bar.open,
                    bar.high,
                    bar.low,
                    bar.close,
                    bar.volume,
                    bar.source,
                    bar.schema_version,
                ),
            )
        except Exception:
            QUESTDB_WRITE_FAIL.labels("insert_bar").inc()
            raise

    async def insert_decision_trace(
        self,
        ts: datetime,
        symbol: str,
        route_id: str,
        regime: str,
        action: str,
        details: str,
    ) -> None:
        if not self._conn:
            raise RuntimeError("not connected")
        sql = """
        INSERT INTO decision_traces (ts, symbol, route_id, regime, action, details)
        VALUES (%s, %s, %s, %s, %s, %s)
        """
        try:
            await self._conn.execute(sql, (ts, symbol, route_id, regime, action, details))
        except Exception:
            QUESTDB_WRITE_FAIL.labels("insert_decision_trace").inc()
            raise

    def _row_from_trace_dict(self, trace: dict) -> tuple:
        ts = datetime.now(UTC)
        sym = str(trace.get("symbol", ""))
        route = trace.get("route") or {}
        route_id = str(route.get("route_id", "")) if isinstance(route, dict) else ""
        reg = trace.get("regime") or {}
        regime_s = str(reg.get("semantic", "")) if isinstance(reg, dict) else ""
        allowed = trace.get("trade_allowed", False)
        action = "trade" if allowed else "blocked"
        details = json.dumps(trace, default=str)
        return (ts, sym, route_id, regime_s, action, details)

    async def insert_decision_trace_dict(self, trace: dict) -> None:
        """Buffer trace rows; flush when batch is full (see ``flush_decision_traces``)."""
        self._trace_buffer.append(trace)
        if len(self._trace_buffer) >= self._batch_max_rows:
            await self.flush_decision_traces()

    async def flush_decision_traces(self) -> None:
        """Write buffered decision traces (call from shutdown or a periodic task)."""
        if not self._conn or not self._trace_buffer:
            return
        batch = self._trace_buffer
        self._trace_buffer = []
        sql = """
        INSERT INTO decision_traces (ts, symbol, route_id, regime, action, details)
        VALUES (%s, %s, %s, %s, %s, %s)
        """
        rows = [self._row_from_trace_dict(t) for t in batch]
        try:
            async with self._conn.cursor() as cur:
                await cur.executemany(sql, rows)
        except Exception:
            self._trace_buffer = batch + self._trace_buffer
            QUESTDB_WRITE_FAIL.labels("flush_decision_traces").inc()
            raise

    async def insert_microservice_events_batch(
        self,
        rows: list[tuple[datetime, str, str, str, str | None, str]],
    ) -> None:
        """Append-only microservice bus audit rows (topic, envelope summary, JSON payload)."""
        if not self._conn or not rows:
            return
        sql = """
        INSERT INTO microservice_events (ts, topic, event_type, trace_id, symbol, payload)
        VALUES (%s, %s, %s, %s, %s, %s)
        """
        async with self._conn.cursor() as cur:
            await cur.executemany(sql, rows)

    async def query_bars(
        self,
        symbol: str,
        start: datetime,
        end: datetime,
        limit: int = 10_000,
        *,
        interval_seconds: int | None = None,
    ) -> list[dict]:
        """Read from ``canonical_bars`` (FB-AP-014). Optional ``interval_seconds`` filter."""
        if not self._conn:
            raise RuntimeError("not connected")
        if interval_seconds is None:
            sql = """
            SELECT ts, symbol, interval_seconds, open, high, low, close, volume, source, schema_version
            FROM canonical_bars
            WHERE symbol = %s AND ts >= %s AND ts <= %s
            ORDER BY ts ASC
            LIMIT %s
            """
            params = (symbol, start, end, limit)
        else:
            sql = """
            SELECT ts, symbol, interval_seconds, open, high, low, close, volume, source, schema_version
            FROM canonical_bars
            WHERE symbol = %s AND ts >= %s AND ts <= %s AND interval_seconds = %s
            ORDER BY ts ASC
            LIMIT %s
            """
            params = (symbol, start, end, interval_seconds, limit)
        async with self._conn.cursor(row_factory=dict_row) as cur:
            await cur.execute(sql, params)
            rows = await cur.fetchall()
        return list(rows)

    async def query_latest_bar(
        self,
        symbol: str,
        *,
        interval_seconds: int,
    ) -> dict | None:
        """Latest row in ``canonical_bars`` for ``symbol`` and ``interval_seconds``, or ``None``."""
        if not self._conn:
            raise RuntimeError("not connected")
        sql = """
        SELECT ts, symbol, interval_seconds, open, high, low, close, volume, source, schema_version
        FROM canonical_bars
        WHERE symbol = %s AND interval_seconds = %s
        ORDER BY ts DESC
        LIMIT 1
        """
        async with self._conn.cursor(row_factory=dict_row) as cur:
            await cur.execute(sql, (symbol, interval_seconds))
            row = await cur.fetchone()
        return dict(row) if row else None

    async def max_canonical_bar_timestamp(
        self,
        symbol: str,
        *,
        interval_seconds: int,
    ) -> datetime | None:
        """Latest ``ts`` in ``canonical_bars`` for ``symbol`` and ``interval_seconds``, or ``None``."""
        if not self._conn:
            raise RuntimeError("not connected")
        sql = """
        SELECT max(ts) AS mx
        FROM canonical_bars
        WHERE symbol = %s AND interval_seconds = %s
        """
        async with self._conn.cursor(row_factory=dict_row) as cur:
            await cur.execute(sql, (symbol, interval_seconds))
            row = await cur.fetchone()
        if not row or row.get("mx") is None:
            return None
        return row["mx"]
