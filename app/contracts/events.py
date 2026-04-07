from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

from pydantic import BaseModel, ConfigDict, Field

from app.contracts.common import DataSource, SemanticRegime

SCHEMA_VERSION = "v1"


class BaseEvent(BaseModel):
    model_config = ConfigDict(extra="forbid")

    timestamp: datetime = Field(default_factory=lambda: datetime.now(UTC))
    schema_version: str = SCHEMA_VERSION
    source: DataSource = DataSource.COINBASE


class BarEvent(BaseEvent):
    symbol: str
    open: float
    high: float
    low: float
    close: float
    volume: float


class TickerEvent(BaseEvent):
    symbol: str
    price: float
    bid: float | None = None
    ask: float | None = None
    volume_24h: float | None = None


class TradeEvent(BaseEvent):
    symbol: str
    trade_id: str | None = None
    side: str | None = None
    price: float
    size: float


class OrderBookEvent(BaseEvent):
    symbol: str
    bids: list[tuple[float, float]]
    asks: list[tuple[float, float]]
    sequence: int | None = None


class RegimeEvent(BaseEvent):
    symbol: str
    raw_state: int
    semantic_state: SemanticRegime
    probabilities: list[float]
    confidence: float


class MemoryFeaturesEvent(BaseEvent):
    symbol: str
    top_k: int
    similarity_score_mean: float
    sentiment_mean: float
    recency_weighted_signal: float
    metadata: dict[str, Any] = Field(default_factory=dict)
