"""
News / sentiment ingestion (FinBERT scoring hooks).

Spec: no raw text affects trades directly — scores feed features only.
V1: pluggable stub; wire NLP in orchestration/retrain flows.
"""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

from pydantic import BaseModel, Field


class NewsItem(BaseModel):
    headline: str
    symbol: str | None = None
    published_at: datetime = Field(default_factory=lambda: datetime.now(UTC))
    url: str | None = None
    raw: dict[str, Any] = Field(default_factory=dict)


class ScoredNews(BaseModel):
    item: NewsItem
    finbert_score: float = 0.0
    frequency_bucket: str = "normal"


def fetch_news_stub(_symbols: list[str]) -> list[NewsItem]:
    """Return empty list until a provider is configured."""
    return []
