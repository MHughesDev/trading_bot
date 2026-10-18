"""N-2.2: baselines beside every candidate, on identical folds."""
import numpy as np
import pytest

from _fam_common import folds, noop_emit, ret_frame, vol_frame
from app import failures
from app.families import baseline as bl


def _cand(definition, df, emit):
    return b"x", {"n": len(df)}


def test_candidate_scorecard_contains_every_baseline_on_identical_folds():
    df = vol_frame()
    f = folds(len(df))
    d = {"target": {"kind": "realized_vol"}}
    _a, m = bl.train_with_baselines(d, df, _cand, noop_emit, folds=f)
    names = [r["name"] for r in m["family_baselines"]]
    assert names == ["mean", "ewma", "ridge", "har", "log_har", "harq"]
    assert m["fold_ids"] == [0, 1, 2]
    for r in m["family_baselines"]:
        assert r["status"] == "ok" and r["folds"] == m["fold_ids"], r["name"]
        assert "qlike" in r["scores"] and np.isfinite(r["scores"]["qlike"])
        assert set(r["per_fold"]) == {"0", "1", "2"}
        assert r["contamination"] is None
        assert len(r["series"]["sigma"]) == len(df)
    by = {r["name"]: r["scores"]["qlike"] for r in m["family_baselines"]}
    assert by["har"] < by["mean"] and by["log_har"] < by["mean"]


def test_baseline_scored_only_on_test_rows_and_series_is_null_elsewhere():
    df = vol_frame()
    rows, _ = bl.evaluate_baselines({"target": {"kind": "realized_vol"}}, df, folds(len(df)), ["har"])
    s = rows[0]["series"]["point"]
    assert all(v is None for v in s[:300])
    assert all(v is not None for v in s[300:600])


def test_harq_refuses_without_quarticity():
    df = vol_frame(with_rq=False)
    d = {"target": {"kind": "realized_vol"}}
    with pytest.raises(bl.MissingRequirement):
        bl.train_harq(d, df, noop_emit)
    assert failures.classify(bl.MissingRequirement("x")) == failures.DATA_ERROR
    rows, _ = bl.evaluate_baselines(d, df, folds(len(df)))
    harq = [r for r in rows if r["name"] == "harq"][0]
    assert harq["status"] == "refused" and harq["scores"] is None and "realized_quarticity" in harq["reason"]
    assert [r for r in rows if r["name"] == "har"][0]["status"] == "ok"


def test_candidate_with_no_baseline_is_refused():
    df = vol_frame()
    with pytest.raises(bl.BaselineMissing):
        bl.train_with_baselines({"target": {"kind": "realized_vol"}}, df, _cand, noop_emit, baselines=[])
    with pytest.raises(bl.BaselineMissing):
        bl.evaluate_baselines({}, df, None, [])


def test_return_baselines_and_standalone_train():
    df = ret_frame(signal=0.02)
    rows, ids = bl.evaluate_baselines({}, df, folds(len(df)))
    by = {r["name"]: r for r in rows}
    assert set(by) == {"mean", "ewma", "ridge"} and ids == [0, 1, 2]
    assert by["ridge"]["scores"]["r2_oos"] > 0.3
    assert by["mean"]["scores"]["r2_oos"] < 0.05
    art, m = bl.train_ridge({}, df, noop_emit)
    assert art[:8] == b"TBNDL001" and m["test_rmse"] > 0


def test_vol_baseline_refuses_non_vol_target():
    with pytest.raises(bl.UnsupportedTarget):
        bl.train_har({}, vol_frame(), noop_emit)


def test_baselines_do_not_leak_test_rows_into_fit():
    df = ret_frame(signal=0.0)
    f = folds(len(df))
    d2 = df.copy()
    d2.loc[f[0]["test_start"]:, "label"] += 100.0
    a, _ = bl.evaluate_baselines({}, df, f[:1], ["mean"])
    b, _ = bl.evaluate_baselines({}, d2, f[:1], ["mean"])
    assert a[0]["series"]["point"][300] == b[0]["series"]["point"][300]
