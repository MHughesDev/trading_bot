"""FB-UX-011: CSV export helpers from API-shaped dicts."""

from __future__ import annotations

from control_plane.csv_export import (
    csv_text_to_utf8_bytes,
    pnl_summary_to_csv_text,
    positions_payload_to_csv_text,
)


def test_positions_payload_ok_rows() -> None:
    text = positions_payload_to_csv_text(
        {
            "ok": True,
            "error": None,
            "adapter": "stub",
            "execution_mode": "paper",
            "positions": [
                {
                    "symbol": "BTC-USD",
                    "quantity": "0.1",
                    "avg_entry_price": "50000",
                    "unrealized_pnl": "10",
                    "mark_price": "50100",
                    "mark_price_source": "kraken_mid",
                    "venue_adapter": "stub",
                }
            ],
        }
    )
    assert "BTC-USD" in text
    assert "stub" in text
    assert "paper" in text


def test_positions_payload_error() -> None:
    text = positions_payload_to_csv_text(
        {
            "ok": False,
            "error": "venue down",
            "positions": [],
            "adapter": "stub",
            "execution_mode": "paper",
        }
    )
    assert "venue down" in text
    assert "false" in text.lower()


def test_pnl_summary_csv() -> None:
    text = pnl_summary_to_csv_text(
        {
            "range": "day",
            "window_start": "2026-01-01T00:00:00+00:00",
            "window_end": "2026-01-02T00:00:00+00:00",
            "realized_pnl_usd": "1.5",
            "unrealized_pnl_usd": "2.5",
            "unrealized_source": "execution_adapter_positions",
            "positions_ok": True,
            "positions_error": None,
            "execution_mode": "paper",
            "ledger": {
                "source_of_truth": "local_jsonl",
                "path": "/tmp/pnl.jsonl",
                "note": "test",
            },
        }
    )
    assert "realized_pnl_usd" in text
    assert "1.5" in text
    assert "/tmp/pnl.jsonl" in text


def test_csv_utf8_bytes() -> None:
    b = csv_text_to_utf8_bytes("€,row")
    assert b.decode("utf-8") == "€,row"
