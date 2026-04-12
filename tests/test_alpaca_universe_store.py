"""FB-AP-020: Alpaca universe SQLite store."""

from __future__ import annotations

from pathlib import Path

from app.runtime.alpaca_universe_store import (
    alpaca_universe_status,
    list_alpaca_universe_rows,
    replace_alpaca_universe_rows,
)


def test_replace_and_list(tmp_path: Path) -> None:
    db = tmp_path / "u.sqlite"
    replace_alpaca_universe_rows(
        db,
        [
            {
                "canonical_symbol": "BTC-USD",
                "alpaca_symbol": "BTCUSD",
                "name": "Bitcoin",
                "asset_class": "crypto",
                "exchange": "CRYPTO",
                "tradable": True,
                "raw_json": "{}",
            }
        ],
    )
    rows, total = list_alpaca_universe_rows(db, limit=10, offset=0, query="btc")
    assert total == 1
    assert rows[0]["canonical_symbol"] == "BTC-USD"
    st = alpaca_universe_status(db)
    assert st["row_count"] == 1
    assert st["last_sync_utc"]
