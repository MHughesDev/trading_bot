"""Aggregate P&L summary for control plane (FB-DASH-05-02)."""

from __future__ import annotations

from datetime import UTC, datetime
from decimal import Decimal
from typing import Any

from app.config.settings import AppSettings
from execution.pnl_ledger import PnlRange, sum_realized_in_window, window_for_range
from execution.portfolio_positions import fetch_portfolio_positions


def _sum_unrealized_from_positions_payload(payload: dict[str, Any]) -> tuple[Decimal | None, str | None]:
    if not payload.get("ok"):
        return None, str(payload.get("error") or "positions unavailable")
    total = Decimal("0")
    saw_any = False
    for p in payload.get("positions") or []:
        raw = p.get("unrealized_pnl")
        if raw is None or raw == "":
            continue
        try:
            total += Decimal(str(raw))
            saw_any = True
        except Exception:
            continue
    if not saw_any:
        return Decimal("0"), None
    return total, None


async def compute_pnl_summary(settings: AppSettings, range_key: PnlRange) -> dict[str, Any]:
    """Realized from local JSONL ledger; unrealized from same path as ``/portfolio/positions``."""
    now = datetime.now(tz=UTC)
    start, end = window_for_range(range_key, now=now)
    realized = sum_realized_in_window(start, end)
    pos = await fetch_portfolio_positions(settings)
    unrealized, pos_err = _sum_unrealized_from_positions_payload(pos)

    out: dict[str, Any] = {
        "range": range_key,
        "window_start": None if start is None else start.isoformat(),
        "window_end": end.isoformat(),
        "realized_pnl_usd": str(realized),
        "unrealized_pnl_usd": None if unrealized is None else str(unrealized),
        "ledger": {
            "source_of_truth": "local_jsonl",
            "path": "data/pnl_ledger.jsonl",
            "note": "Append-only file; future QuestDB or venue backfill — see docs/PNL_LEDGER.MD",
        },
        "unrealized_source": "execution_adapter_positions",
        "positions_ok": bool(pos.get("ok")),
        "positions_error": pos_err,
        "execution_mode": settings.execution_mode,
    }
    return out
