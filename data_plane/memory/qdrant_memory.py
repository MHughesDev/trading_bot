from __future__ import annotations

import math
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Any
from uuid import uuid4

from qdrant_client import QdrantClient
from qdrant_client.http import models as qm

from app.config.settings import QdrantSettings


@dataclass(slots=True)
class MemoryRecord:
    timestamp: datetime
    symbol: str
    headline: str
    sentiment: float
    regime: str | None
    embedding: list[float]


class QdrantMemoryStore:
    def __init__(self, settings: QdrantSettings, vector_size: int = 16) -> None:
        self._settings = settings
        self._collection = settings.collection
        self._enabled = settings.enabled
        self._vector_size = vector_size
        self._client = (
            QdrantClient(host=settings.host, port=settings.port) if settings.enabled else None
        )

    def ensure_collection(self) -> None:
        if not self._enabled or self._client is None:
            return
        collections = self._client.get_collections().collections
        if any(c.name == self._collection for c in collections):
            return
        self._client.create_collection(
            collection_name=self._collection,
            vectors_config=qm.VectorParams(size=self._vector_size, distance=qm.Distance.COSINE),
        )

    def upsert_news(
        self,
        symbol: str,
        headline: str,
        sentiment: float,
        embedding: list[float],
        regime: str | None = None,
        timestamp: datetime | None = None,
    ) -> None:
        if not self._enabled or self._client is None:
            return
        ts = timestamp or datetime.now(UTC)
        payload = {
            "timestamp": ts.isoformat(),
            "symbol": symbol,
            "headline": headline,
            "sentiment": sentiment,
            "regime": regime,
        }
        self._client.upsert(
            collection_name=self._collection,
            points=[
                qm.PointStruct(
                    id=str(uuid4()),
                    vector=embedding[: self._vector_size],
                    payload=payload,
                )
            ],
        )

    def query_context(
        self,
        symbol: str,
        query_vector: list[float],
        top_k: int,
        recency_seconds: int = 3600 * 24,
    ) -> dict[str, float]:
        if not self._enabled or self._client is None:
            return {
                "similarity_score_mean": 0.0,
                "sentiment_mean": 0.0,
                "recency_weighted_signal": 0.0,
                "count": 0.0,
            }

        now = datetime.now(UTC)
        points = self._client.query_points(
            collection_name=self._collection,
            query=query_vector[: self._vector_size],
            query_filter=qm.Filter(
                must=[
                    qm.FieldCondition(key="symbol", match=qm.MatchValue(value=symbol)),
                ]
            ),
            limit=top_k,
            with_payload=True,
            score_threshold=None,
        ).points

        if not points:
            return {
                "similarity_score_mean": 0.0,
                "sentiment_mean": 0.0,
                "recency_weighted_signal": 0.0,
                "count": 0.0,
            }

        sim_vals: list[float] = []
        sent_vals: list[float] = []
        weighted = 0.0
        w_sum = 0.0
        filtered_count = 0
        for p in points:
            payload: dict[str, Any] = p.payload or {}
            ts_raw = payload.get("timestamp")
            if not ts_raw:
                continue
            ts = datetime.fromisoformat(str(ts_raw))
            if ts.tzinfo is None:
                ts = ts.replace(tzinfo=UTC)
            age_sec = max((now - ts).total_seconds(), 0.0)
            if age_sec > recency_seconds:
                continue
            score = float(p.score or 0.0)
            sentiment = float(payload.get("sentiment", 0.0))
            sim_vals.append(score)
            sent_vals.append(sentiment)
            decay = math.exp(-age_sec / max(recency_seconds / 2, 1))
            weighted += sentiment * decay
            w_sum += decay
            filtered_count += 1

        if filtered_count == 0:
            return {
                "similarity_score_mean": 0.0,
                "sentiment_mean": 0.0,
                "recency_weighted_signal": 0.0,
                "count": 0.0,
            }

        return {
            "similarity_score_mean": sum(sim_vals) / len(sim_vals),
            "sentiment_mean": sum(sent_vals) / len(sent_vals),
            "recency_weighted_signal": weighted / w_sum if w_sum else 0.0,
            "count": float(filtered_count),
        }
