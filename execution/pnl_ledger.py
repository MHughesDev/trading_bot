"""Append-only local ledger for realized P&L (USD) — FB-DASH-05.

Future: QuestDB / venue activity can backfill or replace; see docs/PNL_LEDGER.MD.
"""

from __future__ import annotations

import json
import logging
import os
from dataclasses import dataclass
from datetime import UTC, datetime
from decimal import Decimal
from pathlib import Path
from collections import defaultdict
from typing import Any, Literal

logger = logging.getLogger(__name__)

_LEDGER_NAME = "pnl_ledger.jsonl"


def ledger_path(repo_root: Path | None = None) -> Path:
    if os.getenv("NM_MULTI_TENANT_DATA_SCOPING", "").strip().lower() in ("1", "true", "yes"):
        from app.runtime import user_data_paths as user_paths

        return user_paths.pnl_ledger_path()
    root = repo_root or Path(__file__).resolve().parents[1]
    return root / "data" / _LEDGER_NAME


@dataclass(frozen=True)
class RealizedLedgerEntry:
    ts: datetime
    realized_pnl_usd: Decimal
    symbol: str | None
    source: str
    note: str | None = None

    def to_json_line(self) -> str:
        payload = {
            "ts": self.ts.astimezone(UTC).isoformat(),
            "realized_pnl_usd": str(self.realized_pnl_usd),
            "symbol": self.symbol,
            "source": self.source,
            "note": self.note,
        }
        return json.dumps(payload, ensure_ascii=False)


def _parse_ts(raw: str) -> datetime | None:
    try:
        t = datetime.fromisoformat(raw.replace("Z", "+00:00"))
        if t.tzinfo is None:
            t = t.replace(tzinfo=UTC)
        return t.astimezone(UTC)
    except (ValueError, TypeError):
        return None


def _parse_line(line: str) -> RealizedLedgerEntry | None:
    line = line.strip()
    if not line:
        return None
    try:
        obj: dict[str, Any] = json.loads(line)
    except json.JSONDecodeError:
        logger.warning("pnl_ledger: skip invalid json line")
        return None
    ts_raw = obj.get("ts")
    amt_raw = obj.get("realized_pnl_usd")
    if not ts_raw or amt_raw is None:
        return None
    ts = _parse_ts(str(ts_raw))
    if ts is None:
        return None
    try:
        amt = Decimal(str(amt_raw))
    except Exception:
        return None
    sym = obj.get("symbol")
    src = str(obj.get("source") or "unknown")
    note = obj.get("note")
    if note is not None:
        note = str(note)
    return RealizedLedgerEntry(
        ts=ts,
        realized_pnl_usd=amt,
        symbol=str(sym) if sym is not None else None,
        source=src,
        note=note,
    )


def iter_ledger_entries(path: Path | None = None) -> list[RealizedLedgerEntry]:
    p = path or ledger_path()
    if not p.is_file():
        return []
    out: list[RealizedLedgerEntry] = []
    with open(p, encoding="utf-8") as f:
        for line in f:
            e = _parse_line(line)
            if e is not None:
                out.append(e)
    return out


def realized_bucket_series(
    start: datetime | None,
    end: datetime,
    *,
    bucket_seconds: int,
    path: Path | None = None,
) -> list[dict[str, object]]:
    """
    Bucket realized P&L within ``[start, end)`` (``start`` None = no lower bound), UTC.

    Returns rows sorted by bucket with incremental and cumulative USD (strings for JSON).
    """
    bs = max(60, int(bucket_seconds))
    end = end.astimezone(UTC)
    start_eff = start.astimezone(UTC) if start is not None else None
    per_bucket: defaultdict[int, Decimal] = defaultdict(lambda: Decimal("0"))
    for e in iter_ledger_entries(path):
        if e.ts >= end:
            continue
        if start_eff is not None and e.ts < start_eff:
            continue
        epoch = int(e.ts.timestamp())
        b = epoch - (epoch % bs)
        per_bucket[b] += e.realized_pnl_usd
    if not per_bucket:
        return []
    out: list[dict[str, object]] = []
    cum = Decimal("0")
    for b in sorted(per_bucket.keys()):
        inc = per_bucket[b]
        cum += inc
        ts = datetime.fromtimestamp(b, tz=UTC)
        out.append(
            {
                "bucket_start": ts.isoformat(),
                "incremental_usd": str(inc),
                "cumulative_usd": str(cum),
            }
        )
    return out


def sum_realized_in_window(
    start: datetime | None,
    end: datetime,
    *,
    path: Path | None = None,
) -> Decimal:
    """``start`` inclusive (or no lower bound if ``None``), ``end`` exclusive. UTC-normalized."""
    end = end.astimezone(UTC)
    if start is not None:
        start = start.astimezone(UTC)
    total = Decimal("0")
    for e in iter_ledger_entries(path):
        if e.ts >= end:
            continue
        if start is not None and e.ts < start:
            continue
        total += e.realized_pnl_usd
    return total


def append_entry(
    entry: RealizedLedgerEntry,
    *,
    path: Path | None = None,
) -> None:
    p = path or ledger_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    with open(p, "a", encoding="utf-8") as f:
        f.write(entry.to_json_line() + "\n")


PnlRange = Literal["hour", "day", "month", "year", "all"]


def window_for_range(r: PnlRange, *, now: datetime | None = None) -> tuple[datetime | None, datetime]:
    """
    Return ``(start, end)`` for realized aggregation. ``start`` is ``None`` for ``all`` (no lower bound).

    Rolling windows: hour=1h, day=24h, month=30d, year=365d from ``now`` (UTC).
    """
    end = (now or datetime.now(UTC)).astimezone(UTC)
    if r == "all":
        return None, end
    from datetime import timedelta

    delta = {
        "hour": timedelta(hours=1),
        "day": timedelta(days=1),
        "month": timedelta(days=30),
        "year": timedelta(days=365),
    }[r]
    return end - delta, end
