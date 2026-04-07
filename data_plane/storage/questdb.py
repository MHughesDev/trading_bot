"""Append-only writes to QuestDB via PostgreSQL wire protocol."""

from __future__ import annotations

from datetime import datetime

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
    ) -> None:
        self._conninfo = (
            f"host={host} port={port} user={user} password={password} dbname={database}"
        )
        self._conn: psycopg.AsyncConnection | None = None

    async def connect(self) -> None:
        self._conn = await psycopg.AsyncConnection.connect(self._conninfo, autocommit=True)
        await ensure_questdb_schema(self._conn)

    async def aclose(self) -> None:
        if self._conn:
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
