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
    query_embedding: list[float],
    on_features: Callable[[dict[str, float]], object],
    *,
    memory: QdrantNewsMemory | None = None,
) -> None:
    """
    Every `memory_retrieval_interval_seconds`, query Qdrant and pass merged memory features downstream.
    Query embedding is placeholder until news encoder is wired (zeros or last bar embedding).
    """
    mem = memory or QdrantNewsMemory(settings.qdrant_url, settings.memory_qdrant_collection)
    interval = float(settings.memory_retrieval_interval_seconds)
    while True:
        try:
            hits = mem.query_top_k(
                query_embedding,
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
