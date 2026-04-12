"""Client-side CSV export helpers (FB-UX-011) — build UTF-8 CSV from API JSON."""

from __future__ import annotations

import csv
from io import StringIO
from typing import Any

_POSITION_FIELDS = (
    "symbol",
    "quantity",
    "avg_entry_price",
    "unrealized_pnl",
    "mark_price",
    "mark_price_source",
    "venue_adapter",
)


def positions_payload_to_csv_text(data: dict[str, Any]) -> str:
    """
    ``GET /portfolio/positions`` → CSV text.

    One row per open position when ``ok``; includes adapter/mode/error columns on every row.
    When ``ok`` is false, a single row records the error.
    """
    buf = StringIO()
    w = csv.writer(buf)
    header = (
        "adapter",
        "execution_mode",
        "ok",
        "error",
    ) + _POSITION_FIELDS
    w.writerow(header)
    adapter = data.get("adapter", "")
    mode = data.get("execution_mode", "")
    ok = bool(data.get("ok"))
    err = data.get("error") or ""
    if not ok:
        w.writerow([adapter, mode, "false", err] + [""] * len(_POSITION_FIELDS))
        return buf.getvalue()
    for p in data.get("positions") or []:
        row = [adapter, mode, "true", ""]
        for k in _POSITION_FIELDS:
            v = p.get(k)
            row.append("" if v is None else str(v))
        w.writerow(row)
    return buf.getvalue()


def pnl_summary_to_csv_text(summary: dict[str, Any]) -> str:
    """
    ``GET /pnl/summary`` → single-row CSV (flattened scalars + ledger path/note).
    """
    ledger = summary.get("ledger") if isinstance(summary.get("ledger"), dict) else {}
    flat: dict[str, Any] = {
        "range": summary.get("range"),
        "window_start": summary.get("window_start"),
        "window_end": summary.get("window_end"),
        "realized_pnl_usd": summary.get("realized_pnl_usd"),
        "unrealized_pnl_usd": summary.get("unrealized_pnl_usd"),
        "unrealized_source": summary.get("unrealized_source"),
        "positions_ok": summary.get("positions_ok"),
        "positions_error": summary.get("positions_error"),
        "execution_mode": summary.get("execution_mode"),
        "ledger_source_of_truth": ledger.get("source_of_truth"),
        "ledger_path": ledger.get("path"),
        "ledger_note": ledger.get("note"),
    }
    fieldnames = list(flat.keys())
    buf = StringIO()
    w = csv.DictWriter(buf, fieldnames=fieldnames, extrasaction="ignore")
    w.writeheader()
    w.writerow({k: ("" if flat[k] is None else str(flat[k])) for k in fieldnames})
    return buf.getvalue()


def csv_text_to_utf8_bytes(text: str) -> bytes:
    return text.encode("utf-8")
