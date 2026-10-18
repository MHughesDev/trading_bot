"""Purged, clustered permutation importance inputs (Set N N-3.3, MD-09).

The platform's `crates/model-registry/src/importance.rs` owns the *rule*: clusters of
correlated features are permuted together, importance has a standard error across folds,
and a cluster whose interval contains zero has not been shown to matter. It cannot call the
fitted model row by row, so the trainer, which holds the model, produces the per-fold,
per-cluster loss increases and the platform summarises them
(`importance::summarize_fold_importances`).

Purge and embargo are not applied here: the test rows of each fold are the platform's
already-purged `test_start:test_end` ranges, and this module never chooses a row.

Output shape (`metrics["importance_inputs"]`):
    {"features": [names], "clusters": [[feature idx, ...], ...],
     "per_fold": [[increase in fold 0, fold 1, ...] per cluster], "n_repeats": int,
     "folds": number of folds that had test rows}
"""
from __future__ import annotations

import numpy as np


def correlation_clusters(x: np.ndarray, threshold: float = 0.7) -> list[list[int]]:
    """Single-linkage clusters of columns whose |correlation| exceeds `threshold`."""
    d = x.shape[1]
    parent = list(range(d))

    def find(a: int) -> int:
        while parent[a] != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a

    sd = x.std(axis=0)
    for a in range(d):
        for b in range(a + 1, d):
            if sd[a] > 0 and sd[b] > 0:
                r = abs(np.corrcoef(x[:, a], x[:, b])[0, 1])
                if r > threshold:
                    ra, rb = find(a), find(b)
                    if ra != rb:
                        parent[max(ra, rb)] = min(ra, rb)
    groups: dict[int, list[int]] = {}
    for j in range(d):
        groups.setdefault(find(j), []).append(j)
    return sorted(groups.values())


def permutation_increases(predict, x: np.ndarray, y: np.ndarray,
                          test_ranges: list[tuple[int, int]], clusters: list[list[int]],
                          n_repeats: int = 3, seed: int = 0) -> dict:
    """`predict(X) -> 1-D predictions`. Squared-error loss increase per cluster per fold."""
    rng = np.random.default_rng(seed)
    per_fold: list[list[float]] = [[] for _ in clusters]
    used = 0
    for lo, hi in test_ranges:
        if hi - lo < 2:
            continue
        xs, ys = x[lo:hi], y[lo:hi]
        base = float(np.mean((predict(xs) - ys) ** 2))
        used += 1
        for ci, cols in enumerate(clusters):
            inc = 0.0
            for _ in range(n_repeats):
                perm = rng.permutation(len(xs))
                shuffled = xs.copy()
                shuffled[:, cols] = xs[perm][:, cols]  # one shared permutation per cluster
                inc += float(np.mean((predict(shuffled) - ys) ** 2)) - base
            per_fold[ci].append(inc / n_repeats)
    return {"clusters": clusters, "per_fold": per_fold, "n_repeats": n_repeats, "folds": used}
