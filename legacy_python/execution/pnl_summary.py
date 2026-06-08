"""Aggregate P&L summary for control plane (FB-DASH-05-02)."""

from __future__ import annotations

from datetime import UTC, datetime
from decimal import Decimal
from typing import Any

from app.config.settings import AppSettings
from app.runtime.execution_settings_merge import merge_settings_for_execution
from app.runtime.tenant_context import get_current_user_id
from execution.pnl_ledger import (
    PnlRange,
    ledger_path,
    realized_bucket_series,
    sum_realized_in_window,
    window_for_range,
)
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


def compute_pnl_series(
    range_key: PnlRange,
    *,
    bucket_seconds: int = 3600,
    mode: str | None = None,
    now: datetime | None = None,
) -> dict[str, Any]:
    """Cumulative realized P&L series from the JSONL ledger (dashboard chart)."""
    t = now or datetime.now(tz=UTC)
    start, end = window_for_range(range_key, now=t)
    series = realized_bucket_series(start, end, bucket_seconds=bucket_seconds)
    lp = ledger_path()
    return {
        "range": range_key,
        "bucket_seconds": max(60, int(bucket_seconds)),
        "mode": mode,
        "window_start": None if start is None else start.isoformat(),
        "window_end": end.isoformat(),
        "points": series,
        "ledger": {
            "source_of_truth": "local_jsonl",
            "path": str(lp),
        },
    }


async def compute_pnl_summary(settings: AppSettings, range_key: PnlRange) -> dict[str, Any]:
    """Realized from local JSONL ledger; unrealized from same path as ``/portfolio/positions``."""
    now = datetime.now(tz=UTC)
    start, end = window_for_range(range_key, now=now)
    realized = sum_realized_in_window(start, end)
    eff = merge_settings_for_execution(settings, get_current_user_id())
    pos = await fetch_portfolio_positions(eff)
    unrealized, pos_err = _sum_unrealized_from_positions_payload(pos)

    out: dict[str, Any] = {
        "range": range_key,
        "window_start": None if start is None else start.isoformat(),
        "window_end": end.isoformat(),
        "realized_pnl_usd": str(realized),
        "unrealized_pnl_usd": None if unrealized is None else str(unrealized),
        "ledger": {
            "source_of_truth": "local_jsonl",
            "path": str(ledger_path()),
            "note": "Append-only file; future QuestDB or venue backfill — see docs/PNL_LEDGER.MD",
        },
        "unrealized_source": "execution_adapter_positions",
        "positions_ok": bool(pos.get("ok")),
        "positions_error": pos_err,
        "execution_mode": eff.execution_mode,
    }
    return out
