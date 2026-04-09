"""Deterministic dense embeddings from feature dicts (no external model until FinBERT encoder is wired)."""

from __future__ import annotations

import hashlib
import math


def feature_dict_to_embedding(values: dict[str, float], *, dim: int = 64) -> list[float]:
    """
    Stable L2-normalized vector from sorted numeric keys in `values`.
    Same inputs → same vector; suitable for Qdrant cosine search and memory queries.
    """
    keys = sorted(k for k, v in values.items() if isinstance(v, (int, float)) and not isinstance(v, bool))
    if not keys:
        return [0.0] * dim
    raw: list[float] = []
    for k in keys:
        v = float(values[k])
        if math.isnan(v) or math.isinf(v):
            v = 0.0
        # Mix key name for collision resistance across different feature sets
        h = hashlib.sha256(f"{k}:{v:.12g}".encode()).digest()
        raw.append(v)
        raw.append(float(int.from_bytes(h[:4], "big")) / 2**32 - 0.5)
    # Repeat / trim to dim
    out = [0.0] * dim
    for i in range(dim):
        idx = i % len(raw)
        out[i] = raw[idx] * (1.0 + 0.01 * (i // max(len(raw), 1)))
    norm = math.sqrt(sum(x * x for x in out)) + 1e-12
    return [x / norm for x in out]
