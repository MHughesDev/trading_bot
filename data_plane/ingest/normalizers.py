from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

from app.contracts.events import BarEvent, OrderBookEvent, TickerEvent, TradeEvent


def _parse_ts(ts: str | None) -> datetime:
    if not ts:
        return datetime.now(UTC)
    if ts.endswith("Z"):
        ts = ts.replace("Z", "+00:00")
    return datetime.fromisoformat(ts)


def normalize_ticker(msg: dict[str, Any]) -> TickerEvent:
    return TickerEvent(
        timestamp=_parse_ts(msg.get("time")),
        symbol=msg.get("product_id", ""),
        price=float(msg.get("price", 0.0)),
        bid=float(msg["best_bid"]) if msg.get("best_bid") is not None else None,
        ask=float(msg["best_ask"]) if msg.get("best_ask") is not None else None,
        volume_24h=float(msg["volume_24_h"]) if msg.get("volume_24_h") is not None else None,
    )


def normalize_trade(msg: dict[str, Any]) -> TradeEvent:
    return TradeEvent(
        timestamp=_parse_ts(msg.get("time")),
        symbol=msg.get("product_id", ""),
        trade_id=str(msg.get("trade_id")) if msg.get("trade_id") is not None else None,
        side=msg.get("side"),
        price=float(msg.get("price", 0.0)),
        size=float(msg.get("size", 0.0)),
    )


def normalize_bar(msg: dict[str, Any]) -> BarEvent:
    return BarEvent(
        timestamp=_parse_ts(msg.get("start")),
        symbol=msg.get("product_id", ""),
        open=float(msg.get("open", 0.0)),
        high=float(msg.get("high", 0.0)),
        low=float(msg.get("low", 0.0)),
        close=float(msg.get("close", 0.0)),
        volume=float(msg.get("volume", 0.0)),
    )


def normalize_l2(msg: dict[str, Any]) -> OrderBookEvent:
    bids = [(float(px), float(sz)) for px, sz in msg.get("bids", [])]
    asks = [(float(px), float(sz)) for px, sz in msg.get("asks", [])]
    return OrderBookEvent(
        timestamp=_parse_ts(msg.get("time")),
        symbol=msg.get("product_id", ""),
        bids=bids,
        asks=asks,
        sequence=msg.get("sequence_num"),
    )
