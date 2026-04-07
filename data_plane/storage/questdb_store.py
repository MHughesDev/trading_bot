from __future__ import annotations

import logging
from datetime import UTC, datetime

import aiohttp

from app.config.settings import QuestDBSettings
from app.contracts.events import BarEvent, OrderBookEvent, TickerEvent, TradeEvent

logger = logging.getLogger(__name__)


class QuestDBStore:
    """
    Writes normalized market events into QuestDB using ILP over HTTP endpoint.
    """

    def __init__(self, settings: QuestDBSettings) -> None:
        self._host = settings.host
        self._port = settings.ilp_http_port
        self._enabled = settings.enabled
        self._url = f"http://{self._host}:{self._port}/write"

    def _fmt_ts(self, ts: datetime) -> int:
        return int(ts.astimezone(UTC).timestamp() * 1_000_000_000)

    def _line(
        self, measurement: str, tags: dict[str, str], fields: dict[str, float], ts_ns: int
    ) -> str:
        safe_tags = {k: v.replace(" ", "\\ ").replace(",", "\\,") for k, v in tags.items()}
        tags_str = ",".join(f"{k}={v}" for k, v in safe_tags.items())
        fields_str = ",".join(f"{k}={v}" for k, v in fields.items())
        return f"{measurement},{tags_str} {fields_str} {ts_ns}"

    async def write_bar(self, event: BarEvent) -> None:
        line = self._line(
            "bars",
            {"symbol": event.symbol, "source": event.source.value},
            {
                "open": event.open,
                "high": event.high,
                "low": event.low,
                "close": event.close,
                "volume": event.volume,
            },
            self._fmt_ts(event.timestamp),
        )
        await self._send(line)

    async def write_ticker(self, event: TickerEvent) -> None:
        fields = {"price": event.price}
        if event.bid is not None:
            fields["bid"] = event.bid
        if event.ask is not None:
            fields["ask"] = event.ask
        if event.volume_24h is not None:
            fields["volume_24h"] = event.volume_24h
        line = self._line(
            "ticker",
            {"symbol": event.symbol, "source": event.source.value},
            fields,
            self._fmt_ts(event.timestamp),
        )
        await self._send(line)

    async def write_trade(self, event: TradeEvent) -> None:
        line = self._line(
            "trades",
            {"symbol": event.symbol, "source": event.source.value},
            {"price": event.price, "size": event.size},
            self._fmt_ts(event.timestamp),
        )
        await self._send(line)

    async def write_orderbook(self, event: OrderBookEvent) -> None:
        best_bid = event.bids[0][0] if event.bids else 0.0
        best_ask = event.asks[0][0] if event.asks else 0.0
        spread = max(best_ask - best_bid, 0.0) if best_bid and best_ask else 0.0
        line = self._line(
            "orderbook",
            {"symbol": event.symbol, "source": event.source.value},
            {"best_bid": best_bid, "best_ask": best_ask, "spread": spread},
            self._fmt_ts(event.timestamp),
        )
        await self._send(line)

    async def _send(self, line: str) -> None:
        if not self._enabled:
            return
        async with aiohttp.ClientSession() as session:
            async with session.post(
                self._url,
                params={"precision": "n"},
                data=line.encode("utf-8"),
                timeout=10,
            ) as resp:
                if resp.status >= 300:
                    body = await resp.text()
                    logger.error(
                        "questdb_write_failed",
                        extra={"status": resp.status, "body": body, "line": line},
                    )
