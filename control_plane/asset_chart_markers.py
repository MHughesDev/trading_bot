"""Trade marker overlay helpers for Plotly (FB-AP-029) — no Streamlit."""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any


def _parse_ts(raw: str) -> datetime | None:
    try:
        t = datetime.fromisoformat(raw.replace("Z", "+00:00"))
        if t.tzinfo is None:
            t = t.replace(tzinfo=UTC)
        return t.astimezone(UTC)
    except (ValueError, TypeError):
        return None


def _bar_close_series(bars: list[dict[str, Any]]) -> list[tuple[datetime, float]]:
    out: list[tuple[datetime, float]] = []
    for b in bars:
        raw = b.get("ts")
        if raw is None:
            continue
        t = _parse_ts(str(raw))
        if t is None:
            continue
        try:
            c = float(b["close"])
        except (TypeError, ValueError, KeyError):
            continue
        out.append((t, c))
    return out


def _marker_tooltip(m: dict[str, Any]) -> str:
    side = str(m.get("side") or "").lower()
    qty = m.get("quantity", "")
    src = m.get("source", "")
    cid = m.get("correlation_id") or ""
    tip = f"{side.upper()} qty={qty}"
    if src:
        tip += f" · {src}"
    if cid:
        s = str(cid)
        tip += f" · {s[:8]}…" if len(s) > 8 else f" · {s}"
    return tip


def trade_marker_buy_sell_traces(
    markers: list[dict[str, Any]],
    bars: list[dict[str, Any]],
) -> tuple[dict[str, Any], dict[str, Any]]:
    """
    For each marker (sorted by time), **y** = close after advancing bar pointer to last bar with
    ``bar_ts <= marker_ts``. Split into **buy** and **sell** traces for Plotly legend.

    Returns ``(buy_trace, sell_trace)`` each ``{x, y, text}``.
    """
    pairs = _bar_close_series(bars)
    if not pairs or not markers:
        return {"x": [], "y": [], "text": []}, {"x": [], "y": [], "text": []}

    buy_x: list[datetime] = []
    buy_y: list[float] = []
    buy_t: list[str] = []
    sell_x: list[datetime] = []
    sell_y: list[float] = []
    sell_t: list[str] = []

    j = 0
    cur_close = pairs[0][1]
    for m in sorted(markers, key=lambda x: str(x.get("ts") or "")):
        raw_ts = m.get("ts")
        if not raw_ts:
            continue
        mt = _parse_ts(str(raw_ts))
        if mt is None:
            continue
        while j < len(pairs) and pairs[j][0] <= mt:
            cur_close = pairs[j][1]
            j += 1
        tip = _marker_tooltip(m)
        side = str(m.get("side") or "").lower()
        if side == "sell":
            sell_x.append(mt)
            sell_y.append(cur_close)
            sell_t.append(tip)
        else:
            buy_x.append(mt)
            buy_y.append(cur_close)
            buy_t.append(tip)

    return (
        {"x": buy_x, "y": buy_y, "text": buy_t},
        {"x": sell_x, "y": sell_y, "text": sell_t},
    )
