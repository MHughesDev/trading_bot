"""Periodic Qdrant memory retrieval (spec: every 60s) → aggregated feature dict."""

from __future__ import annotations

import asyncio
import logging
import inspect
from collections.abc import Callable

from app.config.settings import AppSettings
from data_plane.memory.qdrant_memory import QdrantNewsMemory

logger = logging.getLogger(__name__)


async def run_memory_retrieval_loop(
    settings: AppSettings,
    symbol: str,
    on_features: Callable[[dict[str, float]], object],
    *,
    query_embedding: list[float] | None = None,
    query_embedding_fn: Callable[[], list[float]] | None = None,
    memory: QdrantNewsMemory | None = None,
) -> None:
    """
    Every `memory_retrieval_interval_seconds`, query Qdrant and pass merged memory features downstream.
    Pass either `query_embedding` or `query_embedding_fn` (preferred for live: embedding from last bar).
    """
    mem = memory or QdrantNewsMemory(settings.qdrant_url, settings.memory_qdrant_collection)
    interval = float(settings.memory_retrieval_interval_seconds)
    while True:
        try:
            qe = query_embedding_fn() if query_embedding_fn is not None else (query_embedding or [0.0] * 64)
            hits = mem.query_top_k(
                qe,
                symbol,
                top_k=settings.memory_top_k,
            )
            feats = mem.memory_features(hits)
            mapped = {
                "mem_sim_mean": feats["sim_mean"],
                "mem_sent_mean": feats["sent_mean"],
                "mem_shock": feats["shock"],
            }
            out = on_features(mapped)
            if inspect.isawaitable(out):
                await out
        except asyncio.CancelledError:
            raise
        except Exception:
            logger.exception("memory retrieval failed for %s", symbol)
        await asyncio.sleep(interval)
