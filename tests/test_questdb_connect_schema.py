from __future__ import annotations

from unittest.mock import AsyncMock, MagicMock

import pytest

from data_plane.storage.questdb import QuestDBWriter
from data_plane.storage.schemas import QUESTDB_CANONICAL_BARS_DDL


@pytest.mark.asyncio
async def test_connect_ensures_canonical_bars_table(monkeypatch):
    fake_conn = MagicMock()
    fake_conn.execute = AsyncMock()

    connect_mock = AsyncMock(return_value=fake_conn)
    monkeypatch.setattr("data_plane.storage.questdb.psycopg.AsyncConnection.connect", connect_mock)

    ensure_mock = AsyncMock()
    monkeypatch.setattr("data_plane.storage.questdb.ensure_questdb_schema", ensure_mock)

    w = QuestDBWriter("h", 8812, "u", "p")
    await w.connect()

    assert w._conn is fake_conn
    fake_conn.execute.assert_awaited_once_with(QUESTDB_CANONICAL_BARS_DDL)
    ensure_mock.assert_awaited_once_with(fake_conn)
