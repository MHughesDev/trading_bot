"""Every walk-forward fold after the first gets its own model (Set U U-5.8)."""
from __future__ import annotations

import pathlib
import sys

import pandas as pd
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from app.fold_models import train_fold_models  # noqa: E402
from app.schemas import FoldSpec  # noqa: E402

HOUR = 3_600_000_000_000


def frame(n=120):
    return pd.DataFrame({"ts_ns": [i * HOUR for i in range(n)], "x": range(n), "label": range(n)})


def fold(index, train_end, cal_end, test_end):
    return FoldSpec(index=index, train_start=0, train_end=train_end, cal_start=train_end,
                    cal_end=cal_end, test_start=cal_end, test_end=test_end)


FOLDS = [fold(0, 20, 30, 50), fold(1, 40, 50, 70), fold(2, 60, 70, 90)]


class Store:
    def __init__(self):
        self.put_keys = []

    def put(self, key, data):
        self.put_keys.append(key)
        return f"file://{key}", f"sha-{data.decode()}", len(data)


def run(folds=FOLDS, train_fn=None):
    seen = []

    def default_train(definition, df, emit):
        seen.append(definition)
        return str(definition["_wf_fold"]["index"]).encode(), {}

    store = Store()
    out = train_fold_models(
        {"objective": "regression", "_checkpoint": {"dir": "x"}, "_wf_fold": "fold 0"},
        frame(),
        folds,
        train_fn or default_train,
        store.put,
        "file:///artifacts/m/v1/",
        lambda *a, **k: None,
    )
    return out, seen, store


def test_each_later_fold_is_fitted_on_its_own_rows_and_stored_beside_the_model():
    out, seen, store = run()
    assert [r["index"] for r in out] == [1, 2], "fold 0 is the version's own model"
    assert [d["_wf_fold"]["index"] for d in seen] == [1, 2]
    assert store.put_keys == ["/artifacts/m/v1/model_fold1.bin", "/artifacts/m/v1/model_fold2.bin"]
    assert [r["sha256"] for r in out] == ["sha-1", "sha-2"], "each fold's own bytes"


def test_a_folds_training_end_is_its_last_training_or_calibration_row():
    out, _, _ = run()
    # Fold 1 saw rows [0, 50): row 49 is the last; its test window starts at 50.
    assert out[0]["trained_through_ns"] == 49 * HOUR
    assert out[1]["trained_through_ns"] == 69 * HOUR
    for r, f in zip(out, FOLDS[1:]):
        assert r["trained_through_ns"] < f.test_start * HOUR, "never into its own test window"


def test_the_main_runs_checkpoint_is_not_reused_by_a_fold():
    _, seen, _ = run()
    assert all("_checkpoint" not in d for d in seen)


def test_one_fold_or_none_trains_nothing_more():
    assert run(folds=FOLDS[:1])[0] == []
    assert run(folds=None)[0] == []


def test_a_fold_that_fails_fails_the_run():
    def boom(definition, df, emit):
        raise ValueError("diverged")

    with pytest.raises(ValueError):
        run(train_fn=boom)
