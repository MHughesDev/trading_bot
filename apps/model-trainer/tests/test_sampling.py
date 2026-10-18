"""N-1.7 sampling modes: CUSUM and strategy events."""
from __future__ import annotations

import pathlib
import sys

import numpy as np
import pandas as pd
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from app import failures, sampling  # noqa: E402
from app.schemas import FoldSpec  # noqa: E402

JUMPS = [100, 180, 260, 340]


def jump_returns(n=400, seed=0):
    rng = np.random.default_rng(seed)
    r = rng.normal(0, 0.0005, n)
    r[0] = 0.0
    for k, j in enumerate(JUMPS):
        r[j] += 0.06 if k % 2 == 0 else -0.06
    return r


def test_cusum_events_land_on_the_planted_jump_bars():
    ev = sampling.cusum_events(jump_returns(), 0.03)
    assert list(ev) == JUMPS


def test_cusum_is_symmetric_a_downward_jump_fires_too():
    r = np.zeros(50)
    r[10] = -0.05
    assert list(sampling.cusum_events(r, 0.02)) == [10]
    r[10] = 0.05
    assert list(sampling.cusum_events(r, 0.02)) == [10]


def test_cusum_accumulates_small_drifts_and_resets_after_an_event():
    r = np.zeros(100)
    r[1:] = 0.004  # 5 bars reach 0.02 (not above); the 6th exceeds it
    ev = sampling.cusum_events(r, 0.02)
    assert list(ev[:3]) == [6, 12, 18]


def test_sigma_threshold_is_causal_truncation_does_not_change_earlier_events():
    r = jump_returns()
    full = sampling.cusum_events(r, "6sigma")
    cut = 250
    part = sampling.cusum_events(r[:cut], "6sigma")
    assert list(part) == [e for e in full if e < cut]
    assert 100 in full and 180 in full  # the jumps still register


def test_future_bars_cannot_change_an_event_decision():
    r = jump_returns()
    r2 = r.copy()
    r2[300:] += 1.0  # rewrite the future
    a = sampling.cusum_events(r, 0.03)
    b = sampling.cusum_events(r2, 0.03)
    assert [e for e in a if e < 300] == [e for e in b if e < 300]


def test_bad_h_is_refused():
    for bad in (0, -1, "fast", "0sigma", None, True):
        with pytest.raises(failures.DataError):
            sampling.cusum_events(np.zeros(5), bad)


def test_strategy_events_use_only_the_entry_timestamps_and_never_a_later_bar():
    ts = np.arange(0, 100) * 60
    entries = [600, 1830, 1830, 5000]  # 1830 falls between bars 30 (1800) and 31 (1860)
    pos = sampling.strategy_event_positions(ts, entries)
    assert list(pos) == [10, 30, 83]  # 5000 -> bar 83 (4980), 1830 -> 30 (not 31), dupes collapse
    assert all(ts[p] <= e for p, e in zip(pos, [600, 1830, 5000]))
    assert list(sampling.strategy_event_positions(ts, [-5])) == []  # before the first bar: dropped


def test_strategy_events_need_sorted_bar_timestamps():
    with pytest.raises(failures.DataError, match="sorted"):
        sampling.strategy_event_positions(np.array([3, 1, 2]), [2])


def frame(n=400):
    r = jump_returns(n)
    close = 100 * np.exp(np.cumsum(r))
    return pd.DataFrame({
        "ts_ns": np.arange(n) * 60_000_000_000, "close": close, "x": np.arange(n, dtype=float),
        "label": r, "label_start": np.arange(n), "label_end": np.arange(n) + 5,
    })


def test_apply_subsamples_the_frame_and_keeps_label_spans():
    df = frame()
    d = {"declaration": {"sampling": {"kind": "cusum", "h": 0.03}}}
    out, folds, info = sampling.apply_to_request(d, df)
    assert info == {"kind": "cusum", "n_rows_before": 400, "n_events": 4}
    assert list(out["x"]) == [float(j) for j in JUMPS]
    assert list(out["label_start"]) == JUMPS and list(out["label_end"]) == [j + 5 for j in JUMPS]
    assert folds is None


def test_apply_strategy_events_selects_exactly_those_rows():
    df = frame()
    ts = df["ts_ns"].to_numpy()
    d = {"declaration": {"sampling": {"kind": "strategy_events", "entry_timestamps": [int(ts[7]), int(ts[200])]}}}
    out, _, info = sampling.apply_to_request(d, df)
    assert list(out["x"]) == [7.0, 200.0] and info["n_events"] == 2


def test_every_bar_and_absent_sampling_leave_the_frame_alone():
    df = frame()
    for decl in ({}, {"sampling": None}, {"sampling": "every_bar"}):
        out, _, info = sampling.apply_to_request({"declaration": decl}, df)
        assert out is df and info is None


def test_unknown_sampling_and_empty_events_are_errors():
    df = frame()
    with pytest.raises(failures.DataError, match="unsupported"):
        sampling.apply_to_request({"declaration": {"sampling": {"kind": "magic"}}}, df)
    with pytest.raises(failures.DataError, match="no events"):
        sampling.apply_to_request({"declaration": {"sampling": {"kind": "cusum", "h": 5.0}}}, df)


def test_fold_ranges_follow_the_rows_they_contained():
    df = frame()
    f = FoldSpec(index=0, train_start=0, train_end=150, cal_start=150, cal_end=200, test_start=200, test_end=400)
    d = {"declaration": {"sampling": {"kind": "cusum", "h": 0.03}}}
    out, folds, _ = sampling.apply_to_request(d, df, [f])
    nf = folds[0]
    assert (nf.train_start, nf.train_end, nf.cal_start, nf.cal_end, nf.test_start, nf.test_end) == (0, 1, 1, 2, 2, 4)
    # every kept row sits in the same fold role it had before
    assert list(out["x"].iloc[nf.train_start:nf.train_end]) == [100.0]
    assert list(out["x"].iloc[nf.cal_start:nf.cal_end]) == [180.0]
    assert list(out["x"].iloc[nf.test_start:nf.test_end]) == [260.0, 340.0]


def test_a_later_volatility_regime_cannot_erase_an_earlier_event():
    rng = np.random.default_rng(5)
    r = np.concatenate([rng.normal(0, 0.001, 200), rng.normal(0, 0.05, 200)])
    r[0] = 0.0
    r[100] = 0.02  # 20 sigma in the quiet regime
    full = sampling.cusum_events(r, "6sigma")
    assert 100 in full
    assert 100 in sampling.cusum_events(r[:150], "6sigma")
