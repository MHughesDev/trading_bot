"""QuestDB trace batching."""

from __future__ import annotations

from unittest.mock import AsyncMock, MagicMock

import pytest

from data_plane.storage.questdb import QuestDBWriter


@pytest.mark.asyncio
async def test_flush_writes_buffered_traces() -> None:
    w = QuestDBWriter("h", 8812, "u", "p", batch_max_rows=2)
    w._conn = MagicMock()
    cur = AsyncMock()
    w._conn.cursor = MagicMock(return_value=cur)
    cur.__aenter__ = AsyncMock(return_value=cur)
    cur.__aexit__ = AsyncMock(return_value=None)
    cur.executemany = AsyncMock()

    await w.insert_decision_trace_dict({"symbol": "BTC-USD", "trade_allowed": False})
    await w.insert_decision_trace_dict({"symbol": "ETH-USD", "trade_allowed": True})
    cur.executemany.assert_awaited_once()
    assert w._trace_buffer == []
