"""How far a trained model's data reached (Set U U-5.4, ADR-0043 decision 5).

A backtest may ask a model version only for decisions after the last instant
its training reached plus an embargo (the label horizon). That instant is a
fact about the rows the model saw, and only the trainer holds those rows, so
the trainer reports it — Rust never infers it from row counts and a window.

"Saw" includes the calibration rows: they choose early stopping and the
calibration map, so information in them is in the model.
"""
from __future__ import annotations

from typing import Iterable, Optional

import pandas as pd


def trained_through_ns(df: pd.DataFrame, folds: Optional[Iterable]) -> Optional[int]:
    """Latest `ts_ns` among the rows the model trained or calibrated on.

    `folds` are the Rust-computed `FoldSpec`s; the trainer fits on the first.
    Without folds the whole frame is the training set. `None` when the frame
    has no `ts_ns` column — unknown is reported as unknown, never guessed.
    """
    if "ts_ns" not in df.columns or len(df) == 0:
        return None
    folds = list(folds or [])
    if folds:
        f = folds[0]
        end = max(int(f.train_end), int(f.cal_end))
        if end <= 0:
            return None
        seen = df["ts_ns"].iloc[: min(end, len(df))]
    else:
        seen = df["ts_ns"]
    return int(seen.max())
