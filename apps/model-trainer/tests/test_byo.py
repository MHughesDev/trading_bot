"""N-2.9 the bring-your-own-model contract: sandbox, code hash, declared features."""
from __future__ import annotations

import hashlib
import pathlib
import sys

import numpy as np
import pandas as pd
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from app import byo, failures, windows  # noqa: E402
from app.byo_interface import ModelInterface  # noqa: E402

GOOD = '''
import numpy as np
from tbot.models import ModelInterface

class Lin(ModelInterface):
    def fit(self, X, y, sample_weight=None):
        A = np.c_[X.to_numpy(), np.ones(len(X))]
        self.coef = np.linalg.lstsq(A, y, rcond=None)[0]
    def predict(self, X):
        return np.c_[X.to_numpy(), np.ones(len(X))] @ self.coef
    def save(self, path):
        np.save(path + "/coef.npy", self.coef)
    @classmethod
    def load(cls, path):
        m = cls(); m.coef = np.load(path + "/coef.npy"); return m
'''

NETWORK = GOOD.replace(
    "    def fit(self, X, y, sample_weight=None):\n",
    "    def fit(self, X, y, sample_weight=None):\n        import socket\n"
    "        try:\n            socket.create_connection(('93.184.216.34', 80), timeout=2)\n"
    "        except Exception:\n            pass  # swallowed: the run must still fail\n",
)

RAW_SOCKET = GOOD.replace(
    "    def fit(self, X, y, sample_weight=None):\n",
    "    def fit(self, X, y, sample_weight=None):\n        import _socket\n        _socket.socket(_socket.AF_INET, _socket.SOCK_STREAM)\n",
)

UNDECLARED_AT_PREDICT = GOOD.replace(
    "    def predict(self, X):\n        return",
    "    def predict(self, X):\n        _ = X['noise']\n        return",
)

WRITES_SNAPSHOT = GOOD.replace(
    "    def fit(self, X, y, sample_weight=None):\n",
    "    def fit(self, X, y, sample_weight=None):\n        open(__file__ + '.pwn', 'w').write('x')\n",
)


def snapshot(tmp_path, code, name="model.py"):
    d = tmp_path / "snap"
    d.mkdir(exist_ok=True)
    (d / name).write_text(code, encoding="utf-8", newline="\n")
    return str(d)


def frame(n=300, seed=0):
    rng = np.random.default_rng(seed)
    x = rng.normal(size=n)
    return pd.DataFrame({"x": x, "noise": rng.normal(size=n), "label": 2.0 * x + 0.1 * rng.normal(size=n)})


def definition(snap, **byo_extra):
    return {"hyperparameters": {"objective": "regression", "seed": 1},
            "byo": {"entry": "model:Lin", "snapshot_dir": snap, "timeout_s": 120, **byo_extra}}


def quiet(*a, **k):
    return None


def test_good_model_trains_and_predicts_in_the_sandbox(tmp_path):
    snap = snapshot(tmp_path, GOOD)
    bundle, m = byo.train(definition(snap), frame(), quiet)
    assert m["test_r2"] > 0.95 and m["family"] == "byo"
    header, _ = windows.read_bundle(bundle)
    assert header["framework"] == "byo" and header["feature_order"] == ["x", "noise"]
    pred = byo.predict_bundle(bundle, frame(50, seed=3))
    assert pred.shape == (50,)
    assert np.corrcoef(pred, frame(50, seed=3)["label"])[0, 1] > 0.95


def test_bundle_records_the_code_hash(tmp_path):
    snap = snapshot(tmp_path, GOOD)
    bundle, m = byo.train(definition(snap), frame(), quiet)
    header, _ = windows.read_bundle(bundle)
    expect = byo.code_hash({"model.py": GOOD.encode()})
    assert header["extra"]["code_hash"] == expect == m["code_hash"]
    # code_hash is the content hash: changing one byte changes it
    assert byo.code_hash({"model.py": GOOD.encode() + b"#"}) != expect


def test_a_submitted_hash_that_does_not_match_the_snapshot_is_refused(tmp_path):
    snap = snapshot(tmp_path, GOOD)
    with pytest.raises(failures.DataError, match="does not match"):
        byo.train(definition(snap, code_hash="0" * 64), frame(), quiet)


def test_a_tampered_bundle_snapshot_is_refused_at_serving(tmp_path):
    import pickle
    import struct
    bundle, _ = byo.train(definition(snapshot(tmp_path, GOOD)), frame(), quiet)
    hlen = struct.unpack("<I", bundle[8:12])[0]
    payload = pickle.loads(bundle[12 + hlen:])
    payload["snapshot"]["model.py"] += b"\n# edited"
    tampered = bundle[: 12 + hlen] + pickle.dumps(payload)
    with pytest.raises(failures.DataError, match="code_hash"):
        byo.predict_bundle(tampered, frame(10))


def test_a_network_call_fails_in_the_sandbox_even_if_the_model_swallows_it(tmp_path):
    with pytest.raises(byo.SandboxViolation, match="network"):
        byo.train(definition(snapshot(tmp_path, NETWORK)), frame(), quiet)


def test_a_raw_low_level_socket_is_blocked_too(tmp_path):
    with pytest.raises(byo.SandboxViolation, match="network"):
        byo.train(definition(snapshot(tmp_path, RAW_SOCKET)), frame(), quiet)


def test_the_snapshot_is_read_only(tmp_path):
    snap = snapshot(tmp_path, WRITES_SNAPSHOT)
    with pytest.raises(byo.SandboxViolation, match="write"):
        byo.train(definition(snap), frame(), quiet)
    assert not (pathlib.Path(snap) / "model.py.pwn").exists()


def test_a_model_that_reads_a_feature_outside_feature_order_fails_at_predict(tmp_path):
    snap = snapshot(tmp_path, UNDECLARED_AT_PREDICT)
    # `noise` exists in the dataset but is not declared, so the model is never handed it.
    with pytest.raises(byo.UndeclaredFeatureRead, match="noise"):
        byo.train(definition(snap, features=["x"]), frame(), quiet)


def test_a_declared_feature_read_succeeds(tmp_path):
    snap = snapshot(tmp_path, UNDECLARED_AT_PREDICT)
    byo.train(definition(snap), frame(), quiet)  # both columns declared: no error


def test_feature_order_records_feature_hash_values(tmp_path):
    hashes = {"x": hashlib.sha256(b"x").hexdigest(), "noise": hashlib.sha256(b"n").hexdigest()}
    bundle, m = byo.train(definition(snapshot(tmp_path, GOOD), feature_hashes=hashes), frame(), quiet)
    header, _ = windows.read_bundle(bundle)
    assert header["feature_order"] == [hashes["x"], hashes["noise"]]
    assert header["extra"]["feature_order_source"] == "feature_hash"
    with pytest.raises(failures.DataError, match="does not cover"):
        byo.train(definition(snapshot(tmp_path, GOOD), feature_hashes={"x": "h"}), frame(), quiet)


def test_one_model_per_fold_plus_a_final_refit(tmp_path):
    from app.schemas import FoldSpec
    folds = [FoldSpec(index=i, train_start=0, train_end=80 + 40 * i, cal_start=80 + 40 * i,
                      cal_end=100 + 40 * i, test_start=100 + 40 * i, test_end=140 + 40 * i).model_dump()
             for i in range(3)]
    bundle, m = byo.train(definition(snapshot(tmp_path, GOOD), folds=folds), frame(), quiet)
    assert [f["index"] for f in m["folds"]] == [0, 1, 2]
    assert all(f["model_files"] == ["coef.npy"] for f in m["folds"])
    assert m["n_refit_rows"] == 180  # train start to the last fold's calibration end
    header, _ = windows.read_bundle(bundle)
    assert header["extra"]["fold_indices"] == [0, 1, 2]
    for i in range(3):
        assert f"fold{i}_test_mse" in m


def test_model_interface_is_abstract_and_missing_methods_are_refused(tmp_path):
    with pytest.raises(TypeError):
        ModelInterface()
    bad = "class Lin:\n    def fit(self, X, y, sample_weight=None): pass\n"
    with pytest.raises(byo.ByoModelError, match="does not implement"):
        byo.train(definition(snapshot(tmp_path, bad)), frame(), quiet)


def test_a_slow_model_times_out(tmp_path):
    slow = GOOD.replace("    def fit(self, X, y, sample_weight=None):\n",
                        "    def fit(self, X, y, sample_weight=None):\n        import time; time.sleep(30)\n")
    with pytest.raises(TimeoutError):
        byo.train(definition(snapshot(tmp_path, slow), timeout_s=3), frame(), quiet)
