"""
Walk-forward train / validation / test splits (initial campaign spec §5).

Timeline is partitioned into ``2 * n_splits + 3`` equal segments. For split ``k``:

- **train**: segments ``0 .. 2k+2`` (inclusive) — expanding window
- **validation**: segment ``2k+3``
- **test**: segment ``2k+4``

Order is always train → validation → test; no shuffling.
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class TripleSplit:
    train: range
    validation: range
    test: range


def triple_splits(n_bars: int, n_splits: int) -> list[TripleSplit]:
    """
    Build ``n_splits`` walk-forward triples. Requires
    ``n_bars >= 2 * n_splits + 3`` (at least one bar per segment).
    """
    if n_splits < 1:
        raise ValueError("n_splits must be >= 1")
    n_parts = 2 * n_splits + 3
    if n_bars < n_parts:
        raise ValueError(
            f"need at least {n_parts} bars for {n_splits} expanding triple splits, got {n_bars}"
        )
    base = n_bars // n_parts
    rem = n_bars % n_parts
    sizes = [base + (1 if i < rem else 0) for i in range(n_parts)]
    bounds: list[int] = [0]
    for s in sizes:
        bounds.append(bounds[-1] + s)
    out: list[TripleSplit] = []
    for k in range(n_splits):
        train_end = bounds[2 * k + 3]
        val_end = bounds[2 * k + 4]
        test_end = bounds[2 * k + 5]
        out.append(
            TripleSplit(
                train=range(bounds[0], train_end),
                validation=range(train_end, val_end),
                test=range(val_end, test_end),
            )
        )
    return out
