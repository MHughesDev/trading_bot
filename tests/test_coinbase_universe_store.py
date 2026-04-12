"""FB-AP-021: Coinbase universe SQLite store."""

from __future__ import annotations

from pathlib import Path

from app.runtime.coinbase_universe_store import (
    coinbase_universe_status,
    list_coinbase_universe_rows,
    replace_coinbase_universe_rows,
)


def test_replace_and_list(tmp_path: Path) -> None:
    db = tmp_path / "c.sqlite"
    replace_coinbase_universe_rows(
        db,
        [
            {
                "product_id": "BTC-USD",
                "base_name": "Bitcoin",
                "quote_name": "US Dollar",
                "product_type": "SPOT",
                "trading_disabled": False,
                "is_disabled": False,
                "raw_json": "{}",
            }
        ],
    )
    rows, total = list_coinbase_universe_rows(db, limit=10, offset=0, query="btc")
    assert total == 1
    assert rows[0]["product_id"] == "BTC-USD"
    st = coinbase_universe_status(db)
    assert st["row_count"] == 1
