"""Qdrant vector memory: news_context_memory collection."""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

import numpy as np
from qdrant_client import QdrantClient
from qdrant_client.models import Distance, FieldCondition, Filter, MatchValue, PointStruct, VectorParams

from app.contracts.regime import SemanticRegime


class QdrantNewsMemory:
    """Store/retrieve news context; embeddings are V1 placeholders until a model is wired."""

    def __init__(self, url: str, collection: str, vector_size: int = 64) -> None:
        self._client = QdrantClient(url=url)
        self._collection = collection
        self._vector_size = vector_size

    @property
    def vector_size(self) -> int:
        return self._vector_size

    def ensure_collection(self) -> None:
        names = [c.name for c in self._client.get_collections().collections]
        if self._collection not in names:
            self._client.create_collection(
                collection_name=self._collection,
                vectors_config=VectorParams(size=self._vector_size, distance=Distance.COSINE),
            )

    def upsert_point(
        self,
        point_id: str,
        embedding: list[float],
        *,
        timestamp: datetime,
        symbol: str,
        sentiment: float,
        regime: SemanticRegime,
        headline: str,
    ) -> None:
        self.ensure_collection()
        payload: dict[str, Any] = {
            "schema_version": 1,
            "timestamp": timestamp.isoformat(),
            "symbol": symbol,
            "sentiment": sentiment,
            "regime": regime.value,
            "headline": headline[:2000],
        }
        vec = np.array(embedding, dtype=np.float32)
        if vec.size != self._vector_size:
            raise ValueError(f"embedding dim {vec.size} != {self._vector_size}")
        point_id_int = abs(hash(point_id)) % (2**63 - 1)
        self._client.upsert(
            collection_name=self._collection,
            points=[
                PointStruct(
                    id=point_id_int,
                    vector=vec.tolist(),
                    payload=payload,
                )
            ],
        )

    def query_top_k(
        self,
        query_embedding: list[float],
        symbol: str,
        *,
        top_k: int = 10,
        max_age_hours: int = 72,
    ) -> list[dict[str, Any]]:
        self.ensure_collection()
        since = datetime.now(UTC).timestamp() - max_age_hours * 3600
        flt = Filter(
            must=[
                FieldCondition(key="symbol", match=MatchValue(value=symbol)),
            ]
        )
        # qdrant-client 1.17+ removed `.search()`; use `query_points` (vector query).
        qres = self._client.query_points(
            collection_name=self._collection,
            query=np.array(query_embedding, dtype=np.float32).tolist(),
            limit=top_k,
            query_filter=flt,
            with_payload=True,
        )
        out: list[dict[str, Any]] = []
        for hit in qres.points:
            pl = hit.payload or {}
            ts = pl.get("timestamp")
            if ts:
                try:
                    tsv = datetime.fromisoformat(ts.replace("Z", "+00:00")).timestamp()
                    if tsv < since:
                        continue
                except ValueError:
                    pass
            out.append({"score": hit.score, "payload": pl})
        return out

    def memory_features(self, hits: list[dict[str, Any]]) -> dict[str, float]:
        """Aggregate similarity, sentiment, recency-weighted signal."""
        if not hits:
            return {"sim_mean": 0.0, "sent_mean": 0.0, "shock": 0.0}
        scores = [float(h["score"]) for h in hits]
        sentiments = [float(h["payload"].get("sentiment", 0.0)) for h in hits]
        return {
            "sim_mean": float(np.mean(scores)),
            "sent_mean": float(np.mean(sentiments)),
            "shock": float(np.std(sentiments)),
        }
