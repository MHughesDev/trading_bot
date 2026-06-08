"""Normalize Kraken WebSocket v1 messages to typed contracts (market data only)."""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

from observability.metrics import NORMALIZER_UNKNOWN

from data_plane.ingest.normalizers import OrderBookLevel2Snapshot, TickerSnapshot, TradeTick


def _parse_kraken_time(v: Any) -> datetime | None:
    if v is None:
        return None
    if isinstance(v, (int, float)):
        return datetime.fromtimestamp(float(v), tz=UTC)
    if isinstance(v, str):
        try:
            return datetime.fromisoformat(v.replace("Z", "+00:00"))
        except ValueError:
            return None
    return None


def normalize_kraken_ws_message(msg: dict[str, Any]) -> TickerSnapshot | TradeTick | OrderBookLevel2Snapshot | None:
    """Map Kraken WS v1 JSON (dict or wrapped array) to contracts."""
    if msg.get("kraken_v1_array"):
        payload = msg.get("payload")
        if not isinstance(payload, list) or len(payload) < 4:
            return None
        ch_name = str(payload[-2]).lower()
        pair = str(payload[-1])
        data = payload[1]
        # Ticker: data is dict with a/b/c/v etc.
        if "ticker" in ch_name and isinstance(data, dict):
            c = data.get("c")
            price: float | None = None
            if isinstance(c, list) and c:
                price = float(c[0])
            bb = data.get("b")
            aa = data.get("a")
            bid = float(bb[0]) if isinstance(bb, list) and bb else None
            ask = float(aa[0]) if isinstance(aa, list) and aa else None
            if price is None and bid is not None and ask is not None:
                price = (bid + ask) / 2.0
            if price is None:
                return None
            vol24 = None
            v = data.get("v")
            if isinstance(v, list) and len(v) > 1:
                vol24 = float(v[1])
            return TickerSnapshot(
                symbol=pair,
                price=price,
                time=datetime.now(UTC),
                bid=bid,
                ask=ask,
                volume_24h=vol24,
                raw={"kraken": data},
            )
        # Trade: data is list of [price, volume, time, side, orderType, misc]
        if "trade" in ch_name and isinstance(data, list) and data:
            tr = data[-1]
            if not isinstance(tr, (list, tuple)) or len(tr) < 3:
                return None
            return TradeTick(
                symbol=pair,
                price=float(tr[0]),
                size=float(tr[1]),
                time=_parse_kraken_time(tr[2]) or datetime.now(UTC),
                side=str(tr[3]).lower() if len(tr) > 3 and tr[3] else None,
                raw={"kraken": tr},
            )
        return None

    event = (msg.get("event") or "").lower()
    if event in ("heartbeat", "pong", "systemstatus", "subscriptionstatus"):
        return None

    NORMALIZER_UNKNOWN.inc()
    return None
