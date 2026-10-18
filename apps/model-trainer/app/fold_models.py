"""One model per walk-forward fold (Set U U-5.8).

The version's own model is fitted on the first fold, so it was trained through
that fold's training-and-calibration rows. A backtest may ask a model only
about decisions after its training end plus the embargo, so a backtest longer
than the first fold's window used to hold for everything before that end — and
had no model trained on the later, larger windows either.

Each later fold gets its own model here: fitted on that fold's rows only, so it
may answer the decisions in that fold's test window. Each is stored beside the
version's model as `model_fold{i}.bin` and reported with its own training end;
the platform records them and a backtest's point-in-time chooser answers every
decision with the latest model trained early enough.

Fold 0 is not refitted: it *is* the version's own model.
"""
from __future__ import annotations

from typing import Callable, Optional

import pandas as pd

from . import pit


def train_fold_models(
    definition: dict,
    df: pd.DataFrame,
    folds: Optional[list],
    train_fn: Callable,
    put: Callable[[str, bytes], tuple[str, str, int]],
    output_prefix: str,
    emit: Callable,
) -> list[dict]:
    """Fit and store one model for each fold after the first.

    Returns one record per fold: its index, stored artifact and training end.
    A fold whose training end is unknown is not stored — a model the
    point-in-time rule cannot place may never answer, so it is not kept. A
    fold that fails to train fails the run: a silently missing fold would hand
    its window to an older model without anyone choosing that.
    """
    folds = list(folds or [])
    out: list[dict] = []
    later = folds[1:]
    for n, fold in enumerate(later, start=1):
        through = pit.trained_through_ns(df, [fold])
        if through is None:
            continue
        fold_def = {
            k: v for k, v in definition.items() if k not in ("_checkpoint", "_resume")
        }
        fold_def["_wf_fold"] = fold.model_dump()
        emit("training_fold", 100.0 * n / (len(later) + 1), {"fold": int(fold.index)})
        artifact_bytes, _metrics = train_fn(fold_def, df, lambda *a, **k: None)
        key = f"{output_prefix.rstrip('/')}/model_fold{int(fold.index)}.bin"
        key = key.replace("file://", "")
        uri, sha256, size = put(key, artifact_bytes)
        out.append(
            {
                "index": int(fold.index),
                "artifact_uri": uri,
                "sha256": sha256,
                "size_bytes": size,
                "trained_through_ns": through,
            }
        )
    return out
