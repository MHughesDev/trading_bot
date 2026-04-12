"""Append-only trade markers for chart overlays (FB-AP-025).

**Source of truth for the chart markers API:** ``data/trade_markers.jsonl`` (gitignored).
Each line is JSON: timestamp, symbol, side, quantity, and provenance. Markers are appended when
the live loop **successfully** submits a risk-signed ``OrderIntent`` (paper or live adapter).

Venue-confirmed fills may be added in a future row; until then, **intent submit** is the stable hook.
"""

from __future__ import annotations

import json
import logging
import os
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

logger = logging.getLogger(__name__)

_MARKERS_NAME = "trade_markers.jsonl"


def markers_path(repo_root: Path | None = None) -> Path:
    if os.getenv("NM_MULTI_TENANT_DATA_SCOPING", "").strip().lower() in ("1", "true", "yes"):
        from app.runtime import user_data_paths as user_paths

        return user_paths.trade_markers_path()
    root = repo_root or Path(__file__).resolve().parents[1]
    return root / "data" / _MARKERS_NAME


@dataclass(frozen=True)
class TradeMarker:
    ts: datetime
    symbol: str
    side: str  # "buy" | "sell"
    quantity: str
    source: str  # e.g. "intent_submit"
    correlation_id: str | None = None
    execution_mode: str | None = None  # paper | live from settings at submit time

    def to_json_line(self) -> str:
        payload = {
            "ts": self.ts.astimezone(UTC).isoformat(),
            "symbol": self.symbol,
            "side": self.side,
            "quantity": self.quantity,
            "source": self.source,
            "correlation_id": self.correlation_id,
            "execution_mode": self.execution_mode,
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


def _parse_line(line: str) -> TradeMarker | None:
    line = line.strip()
    if not line:
        return None
    try:
        obj: dict[str, Any] = json.loads(line)
    except json.JSONDecodeError:
        logger.warning("trade_markers: skip invalid json line")
        return None
    ts_raw = obj.get("ts")
    sym = obj.get("symbol")
    side = obj.get("side")
    qty = obj.get("quantity")
    if not ts_raw or not sym or not side or qty is None:
        return None
    ts = _parse_ts(str(ts_raw))
    if ts is None:
        return None
    src = str(obj.get("source") or "unknown")
    cid = obj.get("correlation_id")
    if cid is not None:
        cid = str(cid)
    em = obj.get("execution_mode")
    if em is not None:
        em = str(em)
    return TradeMarker(
        ts=ts,
        symbol=str(sym).strip(),
        side=str(side).strip().lower(),
        quantity=str(qty),
        source=src,
        correlation_id=cid,
        execution_mode=em,
    )


def iter_markers(
    *,
    symbol: str,
    start: datetime,
    end: datetime,
    path: Path | None = None,
) -> list[TradeMarker]:
    """Markers with ``symbol`` matching and ``start <= ts < end`` (UTC)."""
    sym = symbol.strip()
    start = start.astimezone(UTC)
    end = end.astimezone(UTC)
    p = path or markers_path()
    if not p.is_file():
        return []
    out: list[TradeMarker] = []
    with open(p, encoding="utf-8") as f:
        for line in f:
            m = _parse_line(line)
            if m is None:
                continue
            if m.symbol != sym:
                continue
            if m.ts < start or m.ts >= end:
                continue
            out.append(m)
    out.sort(key=lambda x: x.ts)
    return out


def append_marker(marker: TradeMarker, *, path: Path | None = None) -> None:
    p = path or markers_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    with open(p, "a", encoding="utf-8") as f:
        f.write(marker.to_json_line() + "\n")


def marker_to_api_dict(m: TradeMarker) -> dict[str, Any]:
    return {
        "ts": m.ts.astimezone(UTC).isoformat(),
        "symbol": m.symbol,
        "side": m.side,
        "quantity": m.quantity,
        "source": m.source,
        "correlation_id": m.correlation_id,
        "execution_mode": m.execution_mode,
    }
