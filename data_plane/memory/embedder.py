from __future__ import annotations

import hashlib
import math


def text_to_unit_embedding(text: str, dim: int = 16) -> list[float]:
    """
    Deterministic lightweight embedder for V1 scaffolding.

    Produces a stable unit vector from text without external model dependencies.
    """
    if dim <= 0:
        raise ValueError("dim must be positive")
    digest = hashlib.sha256(text.encode("utf-8")).digest()
    vals = []
    for i in range(dim):
        b = digest[i % len(digest)]
        vals.append((b / 255.0) * 2.0 - 1.0)
    norm = math.sqrt(sum(v * v for v in vals)) or 1.0
    return [v / norm for v in vals]
