"""Walk-forward fold metadata for torch training (FB-FR-P0-03-04)."""

from __future__ import annotations

from forecaster_model.training.walkforward import WalkForwardConfig, walk_forward_indices


def describe_walk_forward_folds(
    n_samples: int,
    *,
    train_len: int,
    val_len: int,
    step: int = 1,
) -> list[dict[str, tuple[int, int]]]:
    """
    Return (train_start, train_end), (val_start, val_end) per fold — no overlap between train and val ranges.

    Use with bar-indexed tensors where ``n_samples`` is the number of rows available for indexing.
    """
    pairs = walk_forward_indices(n_samples, WalkForwardConfig(train_len=train_len, val_len=val_len, step=step))
    out: list[dict[str, tuple[int, int]]] = []
    for tr, va in pairs:
        out.append(
            {
                "train": (tr.start, tr.stop),
                "val": (va.start, va.stop),
            }
        )
    return out
