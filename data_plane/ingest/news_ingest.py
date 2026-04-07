from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Any

from app.contracts.common import SemanticRegime


@dataclass(slots=True)
class NewsItem:
    timestamp: datetime
    symbol: str
    headline: str
    sentiment: float
    regime_hint: SemanticRegime | None = None
    source: str = "synthetic"
    metadata: dict[str, Any] | None = None


class NewsIngestService:
    """
    Placeholder news ingest for V1.

    In production this would connect to one or more curated news streams, run
    FinBERT scoring, and emit typed events to memory storage.
    """

    def __init__(self, symbols: list[str]) -> None:
        self._symbols = symbols
        self._running = False

    async def stream(self) -> AsyncIterator[NewsItem]:
        self._running = True
        while self._running:
            now = datetime.now(UTC)
            for symbol in self._symbols:
                yield NewsItem(
                    timestamp=now,
                    symbol=symbol,
                    headline=f"Heartbeat headline for {symbol}",
                    sentiment=0.0,
                    source="heartbeat",
                    metadata={"synthetic": True},
                )
            await asyncio.sleep(60)

    async def stop(self) -> None:
        self._running = False
