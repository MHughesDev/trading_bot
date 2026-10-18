"""N-1.4 triple-barrier labels on the intrabar high/low path."""
from __future__ import annotations

import pathlib
import sys

import numpy as np
import pandas as pd
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from app import failures  # noqa: E402
from app.labelers import triple_barrier_labels  # noqa: E402

KW = dict(pt=0.02, sl=0.01, vert_bars=3, devol=False)


def flat(n=10, price=100.0):
    return pd.DataFrame({"open": price, "high": price, "low": price, "close": price}, index=range(n)).astype(float)


def test_a_bar_spanning_both_barriers_labels_as_the_stop():
    df = flat()
    df.loc[1, ["high", "low"]] = [103.0, 98.0]  # +2.96% and -2.02% inside one bar
    labels, _ = triple_barrier_labels(df, path="high_low", **KW)
    assert labels[0] == -1.0


def test_a_wick_only_through_the_target_labels_as_the_target():
    df = flat()
    df.loc[1, "high"] = 103.0  # wick above +2%, close unchanged, low unchanged
    labels, _ = triple_barrier_labels(df, path="high_low", **KW)
    assert labels[0] == 1.0
    # the close path cannot see the wick
    close_labels, _ = triple_barrier_labels(df, path="close", **KW)
    assert close_labels[0] == 0.0


def test_a_wick_only_through_the_stop_labels_as_the_stop():
    df = flat()
    df.loc[2, "low"] = 98.5
    labels, _ = triple_barrier_labels(df, path="high_low", **KW)
    assert labels[0] == -1.0


def test_the_first_touching_bar_decides():
    df = flat()
    df.loc[1, "high"] = 103.0  # target first
    df.loc[2, "low"] = 90.0  # stop later
    labels, _ = triple_barrier_labels(df, path="high_low", **KW)
    assert labels[0] == 1.0


def test_a_touch_after_the_vertical_barrier_is_ignored():
    df = flat(12)
    df.loc[5, "high"] = 110.0  # bar 5 is beyond i=0's horizon (bars 1..3)
    labels, _ = triple_barrier_labels(df, path="high_low", **KW)
    assert labels[0] == 0.0 and labels[2] == 1.0  # i=2 sees bar 5


@pytest.mark.parametrize("devol", [True, False])
def test_close_path_result_is_unchanged_on_a_series_with_no_wicks(devol):
    rng = np.random.default_rng(3)
    close = 100 * np.exp(np.cumsum(rng.normal(0, 0.01, 400)))
    df = pd.DataFrame({"open": close, "high": close, "low": close, "close": close})
    kw = dict(pt=2.0 if devol else 0.02, sl=1.0 if devol else 0.01, vert_bars=20, devol=devol)
    a, _ = triple_barrier_labels(df, **kw)  # default path is the close path
    b, _ = triple_barrier_labels(df, path="close", **kw)
    c, _ = triple_barrier_labels(df, path="high_low", **kw)
    assert np.array_equal(a, b)
    assert np.array_equal(a, c)
    assert (a != 0).sum() > 20  # the comparison is not vacuous


def test_high_low_needs_the_columns_and_unknown_paths_are_refused():
    with pytest.raises(failures.DataError, match="high"):
        triple_barrier_labels(flat().drop(columns=["high"]), path="high_low", **KW)
    with pytest.raises(ValueError, match="unknown"):
        triple_barrier_labels(flat(), path="open", **KW)
