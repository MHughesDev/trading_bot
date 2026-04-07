from __future__ import annotations

from dataclasses import dataclass, field
from datetime import UTC, datetime, timedelta

from app.config.settings import MemoryModelSettings
from data_plane.memory.qdrant_memory import QdrantMemoryStore


@dataclass(slots=True)
class MemoryFeatureRetriever:
    store: QdrantMemoryStore
    settings: MemoryModelSettings
    _cache: dict[str, tuple[datetime, dict[str, float]]] = field(default_factory=dict)

    def get_features(self, symbol: str, query_vector: list[float]) -> dict[str, float]:
        now = datetime.now(UTC)
        cached = self._cache.get(symbol)
        if cached is not None:
            ts, payload = cached
            if now - ts < timedelta(seconds=self.settings.refresh_seconds):
                return payload

        payload = self.store.query_context(
            symbol=symbol,
            query_vector=query_vector,
            top_k=self.settings.top_k,
        )
        self._cache[symbol] = (now, payload)
        return payload
