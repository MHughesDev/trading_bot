"""
Branch encoders for forecast / portfolio / execution / risk (human policy spec §10.2).

Implementation lives in `mlp_actor.MultiBranchMLPPolicy._enc`; this module documents the split.
"""

from __future__ import annotations

import numpy as np


def encode_branch(x: list[float], weight_rows: int, rng: np.random.Generator) -> np.ndarray:
    """Generic tanh MLP encoder stub matching the spec’s separate-encoder layout."""
    W = rng.normal(0, 0.1, size=(weight_rows, 32))
    arr = np.asarray(x, dtype=np.float64)
    if len(arr) < W.shape[0]:
        arr = np.pad(arr, (0, W.shape[0] - len(arr)))
    arr = arr[: W.shape[0]]
    return np.tanh(arr @ W)
