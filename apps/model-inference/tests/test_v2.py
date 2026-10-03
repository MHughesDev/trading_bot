"""Serving protocol v2 (Set U U-5.2): typed answers, and nothing zero-filled."""
from __future__ import annotations

import json
import pathlib
import struct
import sys

import numpy as np
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from app import v2  # noqa: E402
from app.predictors import base  # noqa: E402

FEATURES = ["rsi_14", "atr_14", "ret_1"]
MAGIC = b"TBNDL001"


def bundle(inner: bytes, objective: str, extra: dict | None = None) -> bytes:
    header = {
        "schema_version": "tb-bundle-1", "framework": "lightgbm", "objective": objective,
        "feature_order": FEATURES, "scaler": None, "arch": None, "output_kind": "point",
        "quantile_levels": None, "sigma_scaler": None, "calibration": None, "extra": extra or {},
    }
    hb = json.dumps(header).encode()
    return MAGIC + struct.pack("<I", len(hb)) + hb + inner


def train(objective: str) -> bytes:
    import lightgbm as lgb

    rng = np.random.default_rng(7)
    X = rng.normal(size=(400, len(FEATURES)))
    y = (X[:, 0] > 0).astype(float) if objective == "binary" else X[:, 0] * 2.0 + 0.1
    booster = lgb.train({"objective": objective, "verbose": -1, "num_leaves": 7},
                        lgb.Dataset(X, y), num_boost_round=20)
    return booster.model_to_string().encode()


def row(key, **feats):
    return {"key": key, "features": feats}


def test_a_row_missing_a_feature_abstains_naming_it_and_the_rest_still_answer():
    art = bundle(train("regression"), "regression")
    answers = v2.predict_v2(art, [
        row("full", rsi_14=1.0, atr_14=0.2, ret_1=0.0),
        row("short", rsi_14=1.0, ret_1=0.0),
    ], "regressor")
    by_key = {a["key"]: a for a in answers}
    assert by_key["full"]["status"] == "answered"
    assert by_key["short"]["status"] == "abstained"
    assert "atr_14" in by_key["short"]["detail"]
    assert by_key["short"]["outputs"] == {}


def test_a_nan_is_missing_too():
    art = bundle(train("regression"), "regression")
    [a] = v2.predict_v2(art, [row("k", rsi_14=float("nan"), atr_14=0.2, ret_1=0.0)], "regressor")
    assert a["status"] == "abstained" and "rsi_14" in a["detail"]


def test_a_regressor_returns_its_raw_value():
    art = bundle(train("regression"), "regression")
    [hi, lo] = v2.predict_v2(art, [
        row("hi", rsi_14=2.0, atr_14=0.0, ret_1=0.0),
        row("lo", rsi_14=-2.0, atr_14=0.0, ret_1=0.0),
    ], "regressor")
    assert set(hi["outputs"]) == {"value"}
    assert hi["outputs"]["value"] > lo["outputs"]["value"], "a value, not a forecast squashed to 0.5"


def test_a_classifier_returns_its_full_probabilities():
    art = bundle(train("binary"), "classification", extra={"classes": ["down", "up"]})
    [a] = v2.predict_v2(art, [row("k", rsi_14=2.0, atr_14=0.0, ret_1=0.0)], "classifier")
    probs = a["outputs"]["probs"]
    assert probs["labels"] == ["down", "up"]
    assert abs(sum(probs["p"]) - 1.0) < 1e-9
    assert a["outputs"]["class"] == "up"


def test_an_artifact_without_a_feature_order_is_unavailable_not_guessed():
    [a] = v2.predict_v2(train("regression"), [row("k", rsi_14=1.0, atr_14=0.0, ret_1=0.0)], "regressor")
    assert a["status"] == "unavailable" and "feature order" in a["detail"]


def test_v1_no_longer_fills_a_missing_feature_with_zero():
    with pytest.raises(ValueError, match="atr_14"):
        base.features_matrix([{"features": {"rsi_14": 1.0}}], feature_order=["rsi_14", "atr_14"])


# ── transforms (Set U U-5.7) ────────────────────────────────────────────────

SPREAD = {
    "language": "python",
    "entrypoint": "spread",
    "source": "def spread(x):\n    return {'z': (x['a'] - x['b']) / math.sqrt(2)}\n",
}


def test_a_transform_runs_per_row_and_answers_its_outputs():
    [a, b] = v2.run_transform(SPREAD, [row("k1", a=3.0, b=1.0), row("k2", a=1.0, b=1.0)])
    assert a["status"] == "answered" and abs(a["outputs"]["z"] - 2 / 2 ** 0.5) < 1e-12
    assert b["outputs"]["z"] == 0.0


def test_a_transform_that_raises_abstains_naming_the_error():
    [a] = v2.run_transform(SPREAD, [row("k", a=1.0)])
    assert a["status"] == "abstained" and "KeyError" in a["detail"]


def test_a_transform_cannot_import():
    spec = {"language": "python", "entrypoint": "f",
            "source": "def f(x):\n    import os\n    return {'z': 1.0}\n"}
    [a] = v2.run_transform(spec, [row("k", a=1.0)])
    assert a["status"] == "abstained" and "import" in a["detail"].lower()


def test_a_transform_returning_a_non_dict_abstains():
    spec = {"language": "python", "entrypoint": "f", "source": "def f(x):\n    return 3\n"}
    [a] = v2.run_transform(spec, [row("k")])
    assert a["status"] == "abstained" and "not a dict" in a["detail"]
