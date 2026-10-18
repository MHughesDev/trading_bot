"""N-1.5 the Python half of the dataset_build leakage harness."""
from __future__ import annotations

import asyncio
import pathlib
import sys

import numpy as np
import pandas as pd
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
try:  # the worker imports nats at module load; the hook tests never connect
    import nats  # noqa: F401
except ModuleNotFoundError:
    import types

    _stub = types.ModuleType("nats")

    async def _no_connect(*a, **k):
        raise ConnectionError("nats is not installed in this environment")

    _stub.connect = _no_connect
    sys.modules["nats"] = _stub

from app import failures, leakage, worker  # noqa: E402
from app.schemas import FoldSpec, ProgressConfig, TrainRequest  # noqa: E402

H = 5  # label horizon in rows


def frame(n=300):
    i = np.arange(n)
    return pd.DataFrame({"x": i.astype(float), "label": np.sin(i / 7.0),
                         "label_start": i, "label_end": i + H})


def fold(index=0, tr=(0, 100), cal=(100, 120), te=(120, 160)):
    return FoldSpec(index=index, train_start=tr[0], train_end=tr[1], cal_start=cal[0], cal_end=cal[1],
                    test_start=te[0], test_end=te[1])


# ---- (b) label / fold overlap ---------------------------------------------
def test_a_purged_fold_passes():
    # train ends H rows before the test span begins: label_end of the last train row < test start
    c = leakage.label_fold_overlap_check(frame(), [fold(tr=(0, 114), cal=(114, 120), te=(120, 160))])
    assert c["status"] == "pass", c


def test_an_overlapping_label_fold_pair_fails_and_names_the_rows():
    # last train row 119 has label_end 124, inside the test span [120, 164]
    c = leakage.label_fold_overlap_check(frame(), [fold(tr=(0, 120), cal=(120, 120), te=(120, 160))])
    assert c["status"] == "fail"
    v = c["violations"][0]
    assert v["kind"] == "label_overlaps_test_span" and v["n_rows"] == H and v["rows"][0] == 115


def test_a_train_row_inside_the_embargo_after_the_test_fold_fails():
    f = fold(tr=(0, 100), te=(120, 160))
    f2 = f.model_copy(update={"train_start": 0, "train_end": 168})  # includes rows after the test fold
    c = leakage.label_fold_overlap_check(frame(), [f2], embargo_bars=10)
    kinds = {v["kind"] for v in c["violations"]}
    assert "inside_embargo" in kinds or "row_in_train_and_test" in kinds
    assert leakage.label_fold_overlap_check(frame(), [fold(tr=(0, 100), te=(120, 160))], embargo_bars=10)["status"] == "pass"


def test_a_row_in_both_train_and_test_is_caught_even_without_label_spans_overlapping():
    df = frame()
    df["label_start"] = 0
    df["label_end"] = 0  # spans collapsed to one point at 0: no span overlap past row 0
    c = leakage.label_fold_overlap_check(df, [fold(tr=(0, 130), te=(120, 160))])
    assert any(v["kind"] == "row_in_train_and_test" for v in c["violations"])


def test_malformed_label_spans_fail():
    df = frame()
    df.loc[10, "label_end"] = df.loc[10, "label_start"] - 1
    c = leakage.label_fold_overlap_check(df, [fold(tr=(0, 114), te=(120, 160))])
    assert c["status"] == "fail" and c["malformed_label_spans"] == 1


# ---- (a) feature truncation ------------------------------------------------
def causal(df):
    return pd.DataFrame({"ma": df["x"].rolling(5).mean(), "lag": df["x"].shift(1)})


def centered(df):  # uses the future: a centred window
    return pd.DataFrame({"ma": df["x"].rolling(5, center=True).mean()})


def whole_series_scale(df):  # normalises by the full-series max: changes under truncation
    return pd.DataFrame({"z": df["x"] / df["x"].max()})


def test_a_causal_feature_passes_the_truncation_self_test():
    assert leakage.feature_truncation_check(causal, frame())["status"] == "pass"


@pytest.mark.parametrize("fn,col", [(centered, "ma"), (whole_series_scale, "z")])
def test_a_feature_that_changes_when_the_series_is_truncated_fails(fn, col):
    c = leakage.feature_truncation_check(fn, frame())
    assert c["status"] == "fail" and col in c["bad_features"]


# ---- the report and the typed error ---------------------------------------
def test_a_failing_report_raises_a_typed_error_that_is_leakage_detected():
    bad = leakage.build_report(
        [leakage.label_fold_overlap_check(frame(), [fold(tr=(0, 120), cal=(120, 120))])],
        engine_version="fe-3", data_snapshot="snap1",
    )
    assert bad["passed"] is False and bad["failed_checks"] == ["label_fold_overlap"]
    with pytest.raises(leakage.LeakageDetected) as ei:
        leakage.ensure_clean(bad)
    assert failures.classify(ei.value) == failures.LEAKAGE_DETECTED
    assert ei.value.report is bad


def test_the_manifest_records_engine_version_and_data_snapshot():
    r = leakage.check_request({"declaration": {"engine_version": "fe-3"}}, frame(),
                              [fold(tr=(0, 114), cal=(114, 120))], data_snapshot="snap1")
    assert r["checked"] and r["passed"]
    assert r["engine_version"] == "fe-3" and r["data_snapshot"] == "snap1"
    assert [c["name"] for c in r["checks"]] == ["label_fold_overlap", "feature_truncation"]
    assert r["checks"][1]["status"] == "skipped" and r["checks"][1]["reason"]


def test_a_legacy_dataset_without_label_end_records_not_checked():
    df = frame().drop(columns=["label_end"])
    r = leakage.check_request({}, df, [fold()])
    assert r["checked"] is False and "label_end" in r["reason"]
    assert leakage.check_request({}, frame(), None)["checked"] is False


def test_embargo_comes_from_the_derived_declaration():
    d = {"declaration": {"derived": {"embargo_bars": 30}}}
    f = fold(tr=(0, 100), te=(120, 160)).model_copy(update={"train_end": 165})
    with pytest.raises(leakage.LeakageDetected):
        leakage.check_request(d, frame(), [f])


# ---- the hook in worker.run_training --------------------------------------
class FakeStore:
    def __init__(self):
        self.keys = []

    def put(self, key, data):
        self.keys.append(key)
        return "file://" + key, "sha", len(data)


def run_request(monkeypatch, df, folds, definition=None):
    store = FakeStore()
    monkeypatch.setattr(worker, "get_store", lambda: store)
    monkeypatch.setattr(worker, "_load_dataframe", lambda req: df)
    seen = {}

    def train_fn(defn, d, emit):
        seen["rows"] = len(d)
        return b"bin", {"framework_version": "t"}

    monkeypatch.setattr(worker, "_route", lambda *a, **k: train_fn)
    req = TrainRequest(run_id="r", model_id="m", model_kind="regressor", framework="lightgbm",
                       definition=definition or {}, dataset_uri="u", dataset_hash="hash9",
                       output_prefix="file:///tmp/x", progress=ProgressConfig(nats_subject="s"),
                       folds=folds)
    return asyncio.run(worker.run_training(req)), seen


def test_run_training_records_leakage_in_metrics(monkeypatch):
    resp, _ = run_request(monkeypatch, frame(), [fold(tr=(0, 114), cal=(114, 120))])
    assert resp.status == "succeeded", resp.error
    assert resp.metrics["leakage"]["passed"] is True
    assert resp.metrics["leakage"]["data_snapshot"] == "hash9"


def test_run_training_refuses_a_leaking_dataset_before_training(monkeypatch):
    resp, seen = run_request(monkeypatch, frame(), [fold(tr=(0, 120), cal=(120, 120))])
    assert resp.status == "failed" and resp.terminal == failures.LEAKAGE_DETECTED
    assert "rows" not in seen, "no model may be fitted on a leaking dataset"


def test_run_training_on_a_legacy_dataset_records_unchecked(monkeypatch):
    df = frame().drop(columns=["label_start", "label_end"])
    resp, _ = run_request(monkeypatch, df, [fold()])
    assert resp.status == "succeeded"
    assert resp.metrics["leakage"]["checked"] is False and resp.metrics["leakage"]["reason"]


def test_run_training_subsamples_when_the_declaration_says_cusum(monkeypatch):
    n = 400
    r = np.zeros(n)
    r[100], r[250] = 0.06, -0.06
    df = pd.DataFrame({"close": 100 * np.exp(np.cumsum(r)), "x": np.arange(n, dtype=float),
                       "label": r, "label_start": np.arange(n), "label_end": np.arange(n) + 2})
    d = {"declaration": {"sampling": {"kind": "cusum", "h": 0.03}}}
    resp, seen = run_request(monkeypatch, df, None, d)
    assert resp.status == "succeeded", resp.error
    assert seen["rows"] == 2
    assert resp.metrics["sampling"] == {"kind": "cusum", "n_rows_before": 400, "n_events": 2}
