"""Deterministic dense embeddings from feature dicts.

This is a random-projection (hashing-trick) embedding, not a learned semantic model: each
feature *name* maps to a fixed pseudo-random unit direction and the embedding is the
value-weighted sum of those directions, L2-normalized. Seeding from the name (not the value)
is what makes cosine similarity meaningful — two similar feature vectors map to two similar
embeddings (Johnson–Lindenstrauss preserves inner products), unlike a per-value hash which
scatters nearby values to unrelated points. A learned FinBERT/news encoder can replace this
later behind the same signature.
"""

from __future__ import annotations

import hashlib
import math
from functools import lru_cache

import numpy as np


@lru_cache(maxsize=8192)
def _feature_direction(key: str, dim: int) -> tuple[float, ...]:
    """Fixed unit vector in R^dim for a feature *name* (independent of its value)."""
    seed = int.from_bytes(hashlib.blake2b(key.encode(), digest_size=8).digest(), "big")
    rng = np.random.default_rng(seed)
    v = rng.standard_normal(dim)
    n = float(np.linalg.norm(v))
    if n > 0.0:
        v = v / n
    return tuple(v.tolist())


def feature_dict_to_embedding(values: dict[str, float], *, dim: int = 64) -> list[float]:
    """Stable L2-normalized embedding of a numeric feature dict via random projection.

    Same inputs → same vector; suitable for Qdrant cosine search and memory queries. Booleans
    and non-finite values are ignored. An empty/all-non-numeric dict yields a zero vector.
    """
    if dim <= 0:
        return []
    acc = np.zeros(dim, dtype=np.float64)
    any_feat = False
    for k, raw in values.items():
        if not isinstance(raw, (int, float)) or isinstance(raw, bool):
            continue
        v = float(raw)
        if math.isnan(v) or math.isinf(v):
            continue
        acc += v * np.asarray(_feature_direction(k, dim), dtype=np.float64)
        any_feat = True
    if not any_feat:
        return [0.0] * dim
    norm = float(np.linalg.norm(acc))
    if norm <= 1e-12:
        return [0.0] * dim
    return (acc / norm).tolist()
