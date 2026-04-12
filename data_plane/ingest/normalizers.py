"""
Normalize Coinbase Advanced Trade WebSocket messages to typed contracts.

Message shapes vary by channel; unknown messages return None.
"""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

from pydantic import BaseModel, Field

from app.contracts.events import BarEvent
from observability.metrics import NORMALIZER_UNKNOWN


class TickerSnapshot(BaseModel):
    symbol: str
    price: float
    time: datetime
    bid: float | None = None
    ask: float | None = None
    volume_24h: float | None = None
    raw: dict[str, Any] = Field(default_factory=dict)


class TradeTick(BaseModel):
    symbol: str
    price: float
    size: float
    time: datetime
    side: str | None = None
    raw: dict[str, Any] = Field(default_factory=dict)


class OrderBookLevel2Snapshot(BaseModel):
    symbol: str
    bids: list[tuple[float, float]]
    asks: list[tuple[float, float]]
    time: datetime | None = None
    raw: dict[str, Any] = Field(default_factory=dict)


def _parse_time(v: Any) -> datetime | None:
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


def normalize_ws_message(msg: dict[str, Any]) -> (
    BarEvent | TickerSnapshot | TradeTick | OrderBookLevel2Snapshot | None
):
    """Map one WS JSON object to a contract, or None if not recognized."""
    channel = (msg.get("channel") or msg.get("type") or "").lower()
    events = msg.get("events") or []
    if isinstance(events, list) and events:
        # Advanced Trade wraps payloads in events[]
        for ev in events:
            if not isinstance(ev, dict):
                continue
            et = (ev.get("type") or "").lower()
            if "ticker" in channel or et == "ticker":
                parsed = _from_ticker_event(ev, msg)
                if parsed:
                    return parsed
            if "market_trades" in channel or et in ("update", "snapshot"):
                parsed = _from_trade_event(ev, msg)
                if parsed:
                    return parsed
            if "level2" in channel or et in ("l2update", "snapshot"):
                parsed = _from_l2_event(ev, msg)
                if parsed:
                    return parsed
            if "candles" in channel:
                parsed = _from_candle_event(ev, msg)
                if parsed:
                    return parsed
    # Flat ticker
    if msg.get("product_id") and msg.get("price") is not None:
        t = _parse_time(msg.get("time")) or datetime.now(UTC)
        return TickerSnapshot(
            symbol=str(msg["product_id"]),
            price=float(msg["price"]),
            time=t,
            bid=float(msg["bid"]) if msg.get("bid") is not None else None,
            ask=float(msg["ask"]) if msg.get("ask") is not None else None,
            volume_24h=float(msg["volume_24_h"]) if msg.get("volume_24_h") is not None else None,
            raw=msg,
        )
    ch = (msg.get("channel") or "").lower()
    if "heartbeat" not in ch:
        NORMALIZER_UNKNOWN.inc()
    return None


def _from_ticker_event(ev: dict[str, Any], parent: dict[str, Any]) -> TickerSnapshot | None:
    tickers = ev.get("tickers") or []
    if not tickers and isinstance(ev.get("ticker"), dict):
        tickers = [ev["ticker"]]
    for t in tickers:
        if not isinstance(t, dict):
            continue
        pid = t.get("product_id") or t.get("productId")
        price = t.get("price") or t.get("last")
        if not pid or price is None:
            continue
        tm = _parse_time(t.get("time")) or _parse_time(ev.get("time")) or datetime.now(UTC)
        bid = t.get("bid") if t.get("bid") is not None else t.get("best_bid")
        ask = t.get("ask") if t.get("ask") is not None else t.get("best_ask")
        return TickerSnapshot(
            symbol=str(pid),
            price=float(price),
            time=tm,
            bid=float(bid) if bid is not None else None,
            ask=float(ask) if ask is not None else None,
            volume_24h=float(t["volume_24_h"]) if t.get("volume_24_h") is not None else None,
            raw={**parent, "event": ev},
        )
    return None


def _from_trade_event(ev: dict[str, Any], parent: dict[str, Any]) -> TradeTick | None:
    trades = ev.get("trades") or []
    for tr in trades:
        if not isinstance(tr, dict):
            continue
        pid = tr.get("product_id") or tr.get("productId")
        if not pid:
            continue
        tm = _parse_time(tr.get("time")) or datetime.now(UTC)
        return TradeTick(
            symbol=str(pid),
            price=float(tr["price"]),
            size=float(tr.get("size", 0)),
            time=tm,
            side=str(tr["side"]).lower() if tr.get("side") else None,
            raw={**parent, "event": ev},
        )
    return None


def _from_l2_event(ev: dict[str, Any], parent: dict[str, Any]) -> OrderBookLevel2Snapshot | None:
    updates = ev.get("updates") or ev.get("l2_updates") or []
    product_id = ev.get("product_id") or ev.get("productId")
    bids: list[tuple[float, float]] = []
    asks: list[tuple[float, float]] = []
    for u in updates:
        if not isinstance(u, dict):
            continue
        product_id = product_id or u.get("product_id")
        side = (u.get("side") or "").lower()
        price = u.get("price") or u.get("px")
        size = u.get("size") or u.get("qty") or 0
        if price is None:
            continue
        if side == "bid":
            bids.append((float(price), float(size)))
        elif side == "offer" or side == "ask":
            asks.append((float(price), float(size)))
    if product_id and (bids or asks):
        return OrderBookLevel2Snapshot(
            symbol=str(product_id),
            bids=bids,
            asks=asks,
            time=_parse_time(ev.get("time")),
            raw={**parent, "event": ev},
        )
    return None


def _from_candle_event(ev: dict[str, Any], parent: dict[str, Any]) -> BarEvent | None:
    candles = ev.get("candles") or []
    for c in candles:
        if not isinstance(c, dict):
            continue
        pid = c.get("product_id") or c.get("productId")
        if not pid:
            continue
        start = _parse_time(c.get("start"))
        if not start:
            continue
        return BarEvent(
            timestamp=start,
            symbol=str(pid),
            open=float(c["open"]),
            high=float(c["high"]),
            low=float(c["low"]),
            close=float(c["close"]),
            volume=float(c.get("volume", 0)),
            interval_seconds=60,
            source="coinbase",
            schema_version=1,
        )
    return None
