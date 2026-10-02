"""The trainer reports how far a model's data reached (Set U U-5.4)."""
from __future__ import annotations

import pathlib
import sys
from types import SimpleNamespace

import pandas as pd

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from app.pit import trained_through_ns  # noqa: E402

HOUR = 3_600_000_000_000


def frame(n=100):
    return pd.DataFrame({"ts_ns": [i * HOUR for i in range(n)], "x": range(n), "label": range(n)})


def fold(train_end, cal_end, test_end=100):
    return SimpleNamespace(train_start=0, train_end=train_end, cal_start=train_end, cal_end=cal_end,
                           test_start=cal_end, test_end=test_end)


def test_calibration_rows_count_as_seen():
    # Trained on rows [0, 60), calibrated on [60, 75): row 74 is the last seen.
    assert trained_through_ns(frame(), [fold(60, 75)]) == 74 * HOUR


def test_the_test_rows_are_not_seen():
    assert trained_through_ns(frame(), [fold(60, 75)]) < 75 * HOUR


def test_without_folds_the_whole_frame_was_seen():
    assert trained_through_ns(frame(), None) == 99 * HOUR


def test_unknown_is_unknown_not_a_guess():
    assert trained_through_ns(pd.DataFrame({"x": [1, 2]}), [fold(1, 2)]) is None
