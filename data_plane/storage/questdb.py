"""Append-only writes to QuestDB via PostgreSQL wire protocol."""

from __future__ import annotations

import json
from datetime import UTC, datetime

import psycopg
from psycopg.rows import dict_row

from app.contracts.events import BarEvent
from data_plane.storage.schemas import ensure_questdb_schema


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
        await ensure_questdb_schema(self._conn)

    async def aclose(self) -> None:
        if self._conn:
            await self.flush_decision_traces()
            await self._conn.close()
            self._conn = None

    async def insert_bar(self, bar: BarEvent) -> None:
        if not self._conn:
            raise RuntimeError("not connected")
        sql = """
        INSERT INTO bars (ts, symbol, open, high, low, close, volume, source)
        VALUES (%s, %s, %s, %s, %s, %s, %s, %s)
        """
        await self._conn.execute(
            sql,
            (
                bar.timestamp,
                bar.symbol,
                bar.open,
                bar.high,
                bar.low,
                bar.close,
                bar.volume,
                bar.source,
            ),
        )

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
        await self._conn.execute(sql, (ts, symbol, route_id, regime, action, details))

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
        async with self._conn.cursor() as cur:
            await cur.executemany(sql, rows)

    async def query_bars(
        self,
        symbol: str,
        start: datetime,
        end: datetime,
        limit: int = 10_000,
    ) -> list[dict]:
        if not self._conn:
            raise RuntimeError("not connected")
        sql = """
        SELECT ts, symbol, open, high, low, close, volume, source
        FROM bars
        WHERE symbol = %s AND ts >= %s AND ts <= %s
        ORDER BY ts ASC
        LIMIT %s
        """
        async with self._conn.cursor(row_factory=dict_row) as cur:
            await cur.execute(sql, (symbol, start, end, limit))
            rows = await cur.fetchall()
        return list(rows)
