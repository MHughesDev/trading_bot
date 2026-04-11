"""Local JSONL P&L ledger."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from decimal import Decimal
from pathlib import Path

from execution.pnl_ledger import (
    RealizedLedgerEntry,
    append_entry,
    iter_ledger_entries,
    sum_realized_in_window,
    window_for_range,
)


def test_window_for_range() -> None:
    now = datetime(2026, 4, 11, 12, 0, 0, tzinfo=UTC)
    s, e = window_for_range("hour", now=now)
    assert e == now
    assert s == now - timedelta(hours=1)
    s_all, e_all = window_for_range("all", now=now)
    assert s_all is None
    assert e_all == now


def test_sum_realized(tmp_path: Path) -> None:
    p = tmp_path / "l.jsonl"
    t0 = datetime(2026, 4, 11, 10, 0, 0, tzinfo=UTC)
    append_entry(
        RealizedLedgerEntry(
            ts=t0,
            realized_pnl_usd=Decimal("100"),
            symbol="BTC-USD",
            source="test",
        ),
        path=p,
    )
    append_entry(
        RealizedLedgerEntry(
            ts=t0 + timedelta(hours=2),
            realized_pnl_usd=Decimal("-30"),
            symbol="ETH-USD",
            source="test",
        ),
        path=p,
    )
    mid = t0 + timedelta(hours=1)
    assert sum_realized_in_window(t0, mid, path=p) == Decimal("100")
    assert sum_realized_in_window(None, t0 + timedelta(days=1), path=p) == Decimal("70")
    assert len(iter_ledger_entries(path=p)) == 2
