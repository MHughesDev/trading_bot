"""
News / sentiment ingestion (FinBERT scoring hooks).

Spec: no raw text affects trades directly — scores feed features only.
RSS sources are configurable; dedup by stable hash of guid/link/title.
"""

from __future__ import annotations

import logging
from collections import OrderedDict
from datetime import UTC, datetime
from typing import Any

import httpx
from pydantic import BaseModel, Field

from data_plane.ingest.rss_news import dedup_key, fetch_feed_items
from data_plane.ingest.sentiment_nlp import score_headline_finbert

logger = logging.getLogger(__name__)

# Process-local dedup (newest keys retained; bounded)
_MAX_DEDUP_KEYS = 8_000


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


_seen_dedup: OrderedDict[str, None] = OrderedDict()


def _remember_seen(keys: list[str]) -> None:
    for k in keys:
        if k in _seen_dedup:
            _seen_dedup.move_to_end(k)
        else:
            _seen_dedup[k] = None
        while len(_seen_dedup) > _MAX_DEDUP_KEYS:
            _seen_dedup.popitem(last=False)


def _filter_new_items(items: list[NewsItem]) -> list[NewsItem]:
    new: list[NewsItem] = []
    keys: list[str] = []
    for it in items:
        key = dedup_key(it.headline, it.url, it.raw.get("guid"))
        if key in _seen_dedup:
            continue
        keys.append(key)
        new.append(it)
    _remember_seen(keys)
    return new


async def fetch_news_for_symbols(
    symbols: list[str],
    *,
    rss_feeds: list[str],
    timeout: float = 15.0,
) -> list[NewsItem]:
    """
    Fetch headlines from configured RSS/Atom URLs; map rough symbol hints from title text.
    When ``rss_feeds`` is empty, returns no items (neutral sentiment path).
    """
    if not rss_feeds:
        return []
    upper_syms = [s.replace("-", "").upper() for s in symbols]
    out: list[NewsItem] = []
    async with httpx.AsyncClient(
        headers={"User-Agent": "NautilusMonster/3 (news ingest)"},
        follow_redirects=True,
    ) as client:
        for url in rss_feeds:
            try:
                rows = await fetch_feed_items(client, url, timeout=timeout)
            except Exception:
                logger.exception("RSS fetch failed for %s", url)
                continue
            for title, link, guid, pub in rows:
                hint: str | None = None
                t_up = title.upper()
                for i, raw in enumerate(symbols):
                    base = raw.split("-")[0].upper()
                    if base and base in t_up:
                        hint = symbols[i] if i < len(symbols) else None
                        break
                if hint is None and upper_syms:
                    hint = symbols[0]
                out.append(
                    NewsItem(
                        headline=title,
                        symbol=hint,
                        published_at=pub,
                        url=link or None,
                        raw={"guid": guid, "feed": url},
                    )
                )
    return _filter_new_items(out)


def fetch_news_stub(symbols: list[str]) -> list[NewsItem]:
    """Backward-compatible name: empty when RSS not configured in sync callers."""
    return []


async def aggregate_sentiment_for_symbols_async(
    symbols: list[str],
    *,
    use_finbert: bool,
    rss_feeds: list[str],
    fetch_timeout_seconds: float = 15.0,
) -> dict[str, float]:
    """
    FinBERT + frequency + shock from RSS items.
    When no items or FinBERT unavailable, returns neutral zeros (features still wired).
    """
    import numpy as np

    items = await fetch_news_for_symbols(
        symbols, rss_feeds=rss_feeds, timeout=fetch_timeout_seconds
    )
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


def aggregate_sentiment_for_symbols(
    symbols: list[str],
    *,
    use_finbert: bool,
    rss_feeds: list[str] | None = None,
) -> dict[str, float]:
    """Sync helper for tests and scripts (no RSS unless feeds passed)."""
    feeds = rss_feeds or []
    if not feeds:
        return {"finbert_score": 0.0, "news_count_per_hour": 0.0, "sentiment_shock": 0.0}
    import asyncio

    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return asyncio.run(
            aggregate_sentiment_for_symbols_async(
                symbols, use_finbert=use_finbert, rss_feeds=feeds
            )
        )
    raise RuntimeError("Use aggregate_sentiment_for_symbols_async from async code")
