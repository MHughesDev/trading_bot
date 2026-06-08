"""Time-respecting walk-forward index splits (no lookahead)."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class WalkForwardConfig:
    train_len: int
    val_len: int
    step: int = 1


def walk_forward_indices(
    n_samples: int,
    cfg: WalkForwardConfig,
) -> list[tuple[range, range]]:
    """
    Yield (train_range, val_range) where val indices are strictly after train indices.

    Requires n_samples >= train_len + val_len.
    """
    out: list[tuple[range, range]] = []
    t0 = 0
    while True:
        tr_end = t0 + cfg.train_len
        va_start = tr_end
        va_end = va_start + cfg.val_len
        if va_end > n_samples:
            break
        out.append((range(t0, tr_end), range(va_start, va_end)))
        t0 += cfg.step
    return out
