"""Worker wiring for Set N: declaration/family reach the definition, run_evaluation attaches
family baselines + series, GARCH eval uses the conditional sigma."""
import asyncio
import sys
import types

import numpy as np
import pandas as pd
import pytest

import _fam_common  # noqa: F401

pytest.importorskip("pyarrow")

if "nats" not in sys.modules:  # the NATS client is optional in tests
    sys.modules["nats"] = types.ModuleType("nats")

from app import artifact_store, worker  # noqa: E402
from app.families import registry, vol  # noqa: E402
from app.schemas import EvalRequest, FoldSpec, TrainRequest  # noqa: E402


@pytest.fixture()
def store(tmp_path, monkeypatch):
    monkeypatch.setenv("ARTIFACT_STORE", "fs")
    monkeypatch.setenv("ARTIFACT_STORE_PATH", str(tmp_path / "art"))
    st = artifact_store.ArtifactStore()
    monkeypatch.setattr(artifact_store, "_STORE", st)
    monkeypatch.setattr(worker, "get_store", lambda: st)
    return st


def _frame(n=1200):
    from _fam_common import sim_garch

    r = sim_garch(n, seed=21)
    return pd.DataFrame({"ret": r, "label": np.append(r[1:], 0.0)})


def _put_frame(store, df, name):
    import io

    b = io.BytesIO()
    df.to_parquet(b)
    uri, _, _ = store.put(name, b.getvalue())
    return uri


def test_train_request_declaration_and_family_reach_the_train_function(store, monkeypatch):
    seen = {}

    def fake_train(definition, df, emit):
        seen["definition"] = dict(definition)
        return b"bundle", {"framework_version": "t"}

    monkeypatch.setattr(registry, "_import_entrypoint", lambda path: fake_train)
    uri = _put_frame(store, _frame(300), "ds/train.parquet")
    decl = {"sequence": {"window": 64, "stride": 1}, "derived": {"purge_bars": 24, "embargo_bars": 24, "horizon_bars": 24}}
    req = TrainRequest(
        run_id="r1", model_id="m", model_kind="forecaster", framework="", definition={},
        dataset_uri=uri, dataset_hash="h", output_prefix="out/r1", progress={"nats_subject": "s"},
        declaration=decl, family={"family": "regime", "variant": "hmm"},
    )
    resp = asyncio.run(worker.run_training(req))
    assert resp.status == "succeeded", resp.error
    assert seen["definition"]["declaration"] == decl
    assert seen["definition"]["family"] == {"family": "regime", "variant": "hmm"}


def test_train_request_without_family_uses_the_legacy_framework_path(store, monkeypatch):
    got = {}
    monkeypatch.setattr(registry, "_import_entrypoint", lambda path: (got.setdefault("p", path), lambda d, df, e: (b"b", {}))[1])
    uri = _put_frame(store, _frame(300), "ds/train2.parquet")
    req = TrainRequest(run_id="r2", model_id="m", model_kind="classifier", framework="lightgbm", definition={},
                       dataset_uri=uri, dataset_hash="h", output_prefix="out/r2", progress={"nats_subject": "s"})
    assert asyncio.run(worker.run_training(req)).status == "succeeded"
    assert got["p"] == "app.adapters.lightgbm_adapter:train"


def _eval_req(store, run_baselines=True):
    df = _frame()
    d = {"output": {"quantile_levels": [0.05, 0.25, 0.5, 0.75, 0.95]}, "family": {"family": "vol", "variant": "garch"}}
    art, _ = vol.train_garch(d, df.iloc[:700], lambda *a, **k: None)
    a_uri, _, _ = store.put("m/model.bin", art)
    ds_uri = _put_frame(store, df, "ds/eval.parquet")
    folds = [FoldSpec(index=i, train_start=0, train_end=700 + 100 * i, cal_start=700 + 100 * i, cal_end=700 + 100 * i,
                      test_start=700 + 100 * i, test_end=800 + 100 * i) for i in range(3)]
    return EvalRequest(eval_id="e", model_id="m", version=1, model_kind="forecaster", artifact_uri=a_uri,
                       artifact_hash="x", dataset_uri=ds_uri, dataset_hash="h", definition=d, folds=folds,
                       run_baselines=run_baselines), df


def test_garch_eval_attaches_family_baselines_series_and_uses_conditional_sigma(store):
    req, df = _eval_req(store)
    resp = asyncio.run(worker.run_evaluation(req))
    assert resp.status == "succeeded", resp.error
    m = resp.metrics
    assert isinstance(m["baselines"], dict) and "naive" in m["baselines"]  # untouched legacy dict
    assert m["fold_ids"] == [0, 1, 2]
    rows = {r["name"]: r for r in m["family_baselines"]}
    assert {"mean", "ewma", "ridge"} <= set(rows)
    s = m["series"]
    assert s["target_kind"] == "vol"
    n = len(s["realized"])
    assert n == len(s["point"]) == len(s["sigma"]) and n <= 20000
    assert n == len(df) - 29  # the 30-bar warm-up prefix is dropped, not fabricated
    for r in rows.values():
        assert r["series"] is not None and len(r["series"]["point"]) == n
        assert r["folds"] == m["fold_ids"] and r["contamination"] is None
    sig = np.array(s["sigma"], float)
    assert np.all(np.isfinite(sig)) and sig.std() / sig.mean() > 0.03  # conditional, not a constant
    assert m["candidate"]["folds"] == [0, 1, 2]
    assert resp.scorecard["family_baselines"] == m["family_baselines"]


def test_evaluation_without_baselines_is_refused(store):
    req, _ = _eval_req(store, run_baselines=False)
    resp = asyncio.run(worker.run_evaluation(req))
    assert resp.status == "failed" and resp.terminal == "data_error"
    assert "baseline" in resp.error.lower()
