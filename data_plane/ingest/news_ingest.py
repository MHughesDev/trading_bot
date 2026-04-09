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


def aggregate_sentiment_for_symbols(
    symbols: list[str],
    *,
    use_finbert: bool,
) -> dict[str, float]:
    """
    FinBERT + frequency + shock from `fetch_news_stub` items.
    When no items or FinBERT unavailable, returns neutral zeros (features still wired).
    """
    import numpy as np

    from data_plane.ingest.sentiment_nlp import score_headline_finbert

    items = fetch_news_stub(symbols)
    if not items:
        return {"finbert_score": 0.0, "news_count_per_hour": 0.0, "sentiment_shock": 0.0}
    scores: list[float] = []
    for it in items:
        scores.append(score_headline_finbert(it.headline) if use_finbert else 0.0)
    arr = np.array(scores, dtype=np.float64)
    return {
        "finbert_score": float(np.mean(arr)),
        "news_count_per_hour": float(len(items)),
        "sentiment_shock": float(np.std(arr)) if arr.size > 1 else 0.0,
    }
