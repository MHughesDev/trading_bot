"""Serving protocol v2: typed, named inputs and outputs (Set U U-5.2, ADR-0043).

v1 answered every model with a forecast and filled any feature it was not given
with ``0.0`` — so a classifier's probabilities, a regressor's value and an
embedding all arrived squeezed into ``direction/magnitude/confidence``, and a
missing input became a confident answer about a world where that input was zero
(FEAT-007 §1 defect 4).

v2 answers in the shape of the model:

* each request row carries named feature values; a row missing any feature the
  model was trained on **abstains, naming the features** — nothing is filled;
* each answer is a dict of named outputs in plain JSON — numbers, labels,
  ``{"labels", "p"}`` for probabilities, ``{"levels", "values"}`` for quantiles,
  lists for vectors — which the Rust provider types against the version's
  contract;
* an artifact that cannot be loaded or decoded makes every row ``unavailable``
  with the reason — never a made-up forecast.

Status per row: ``answered`` | ``abstained`` | ``unavailable``.
"""
from __future__ import annotations

import math
import os
import pickle
import tempfile
from typing import Any

import numpy as np

from .predictors import base
from .predictors import bundle as bundle_mod


class Unavailable(Exception):
    """The artifact could not answer at all."""


def _finite(x: Any) -> bool:
    try:
        return x is not None and math.isfinite(float(x))
    except (TypeError, ValueError):
        return False


def _raw_scores(framework: str, inner: bytes, X: np.ndarray, want_proba: bool) -> np.ndarray:
    """The model's raw output for each row: a score, a value, or class
    probabilities (2-D) when ``want_proba`` and the model has them."""
    fw = (framework or "").split("_")[0]
    if fw == "lightgbm":
        import lightgbm as lgb

        try:
            booster = lgb.Booster(model_str=inner.decode("utf-8"))
        except Exception:  # noqa: BLE001
            path = None
            try:
                with tempfile.NamedTemporaryFile(suffix=".txt", delete=False) as tmp:
                    tmp.write(inner)
                    path = tmp.name
                booster = lgb.Booster(model_file=path)
            finally:
                if path and os.path.exists(path):
                    os.unlink(path)
        return np.asarray(booster.predict(X), dtype=float)
    if fw == "xgboost":
        import xgboost as xgb

        booster = xgb.Booster()
        booster.load_model(bytearray(inner))
        return np.asarray(booster.predict(xgb.DMatrix(X)), dtype=float)
    if fw == "sklearn":
        model = pickle.loads(inner)  # noqa: S301 — our own trainer's artifact
        if want_proba and hasattr(model, "predict_proba"):
            return np.asarray(model.predict_proba(X), dtype=float)
        return np.asarray(model.predict(X), dtype=float)
    raise Unavailable(f"framework '{framework}' is not served by v2")


def _probs(raw: np.ndarray, labels: list[str]) -> list[dict]:
    """Per-row ``{"labels", "p"}`` from a binary score or a probability matrix."""
    out = []
    if raw.ndim == 1:
        labels = labels or ["0", "1"]
        for p in raw:
            p1 = float(np.clip(p, 0.0, 1.0))
            out.append({"labels": labels[:2], "p": [1.0 - p1, p1]})
    else:
        labels = labels or [str(i) for i in range(raw.shape[1])]
        for row in raw:
            s = float(np.sum(row)) or 1.0
            out.append({"labels": labels[: raw.shape[1]], "p": [float(v) / s for v in row]})
    return out


def _forecaster_outputs(inner: bytes, header: dict, rows: list[dict], model_kind: str) -> list[dict]:
    """A forecaster answers through its existing framework predictor; v2 only
    reshapes the result into named outputs."""
    from .predictors import _BY_FRAMEWORK

    fn = _BY_FRAMEWORK.get(header.get("framework"))
    if fn is None:
        raise Unavailable(f"framework '{header.get('framework')}' is not served")
    forecasts = fn(inner, [{"features": r["features"]} for r in rows], model_kind, "1h", header)
    if forecasts is None:
        raise Unavailable("the predictor returned nothing")
    out = []
    for f in forecasts:
        o: dict[str, Any] = {"direction": f.direction, "confidence": float(f.confidence)}
        median = f.median_return if f.median_return is not None else float(f.magnitude)
        o["median_return"] = float(median)
        if f.sigma is not None:
            o["sigma"] = float(f.sigma)
        if f.quantile_levels and f.quantiles_return:
            o["quantiles"] = {"levels": list(f.quantile_levels), "values": list(f.quantiles_return)}
        out.append(o)
    return out


def predict_v2(artifact_bytes: bytes, rows: list[dict], model_kind: str) -> list[dict]:
    """Answer every row. Each answer: ``{"key", "status", "outputs", "detail"}``."""
    header, inner = bundle_mod.read_bundle(artifact_bytes)
    if header is None or not header.get("feature_order"):
        return [
            {"key": r.get("key"), "status": "unavailable", "outputs": {},
             "detail": "the artifact is not a self-describing bundle, so its training feature "
                       "order is unknown; v2 will not guess one"}
            for r in rows
        ]
    order = list(header["feature_order"])
    answers: list[dict | None] = [None] * len(rows)
    ready: list[int] = []
    for i, r in enumerate(rows):
        feats = r.get("features") or {}
        missing = [k for k in order if not _finite(feats.get(k))]
        if missing:
            answers[i] = {"key": r.get("key"), "status": "abstained", "outputs": {},
                          "detail": "missing required feature(s): " + ", ".join(missing)}
        else:
            ready.append(i)
    if ready:
        live = [rows[i] for i in ready]
        try:
            if model_kind == "forecaster":
                outputs = _forecaster_outputs(inner, header, live, model_kind)
            else:
                X = np.asarray([[float(r["features"][k]) for k in order] for r in live], dtype=np.float64)
                X = base.apply_scaler(X, header.get("scaler")).astype(np.float32)
                extra = header.get("extra") or {}
                classes = [str(c) for c in (extra.get("classes") or [])]
                if model_kind == "classifier":
                    raw = _raw_scores(header.get("framework", ""), inner, X, want_proba=True)
                    outputs = []
                    for pr in _probs(raw, classes):
                        best = int(np.argmax(pr["p"]))
                        outputs.append({"class": pr["labels"][best], "probs": pr})
                elif model_kind in ("regressor", "signal_ranker", "risk_sizing"):
                    raw = np.atleast_1d(_raw_scores(header.get("framework", ""), inner, X, want_proba=False))
                    name = {"regressor": "value", "signal_ranker": "score", "risk_sizing": "size_fraction"}[model_kind]
                    outputs = [{name: float(v)} for v in raw]
                else:
                    raise Unavailable(f"model kind '{model_kind}' is not served by v2")
            for i, o in zip(ready, outputs):
                answers[i] = {"key": rows[i].get("key"), "status": "answered", "outputs": o, "detail": None}
        except Unavailable as e:
            for i in ready:
                answers[i] = {"key": rows[i].get("key"), "status": "unavailable", "outputs": {}, "detail": str(e)}
        except Exception as e:  # noqa: BLE001
            for i in ready:
                answers[i] = {"key": rows[i].get("key"), "status": "unavailable", "outputs": {},
                              "detail": f"the model failed: {e}"}
    return [a for a in answers if a is not None]


class MissingFeatures(ValueError):
    """v1: a request that lacks features the model was trained on."""


# --------------------------------------------------------------------------- #
# Transforms (Set U U-5.7): custom code with a declared contract, no training.
# --------------------------------------------------------------------------- #

_SAFE_BUILTINS = {
    name: __builtins__[name] if isinstance(__builtins__, dict) else getattr(__builtins__, name)
    for name in (
        "abs", "all", "any", "bool", "dict", "enumerate", "float", "int", "len", "list",
        "max", "min", "pow", "range", "round", "sorted", "str", "sum", "tuple", "zip",
        "isinstance", "ValueError", "ZeroDivisionError",
    )
}


def run_transform(spec: dict, rows: list[dict]) -> list[dict]:
    """Run a transform's ``entrypoint(inputs) -> dict`` once per row.

    Restricted builtins, no ``import``, ``math`` provided: this stops accidents,
    not an adversary — a transform is owner-registered code, the same trust as
    the pickled artifacts this service already loads. A row whose call raises or
    returns something that is not a dict abstains with the reason; nothing is
    filled in.
    """
    if (spec or {}).get("language", "python") != "python":
        return [{"key": r.get("key"), "status": "unavailable", "outputs": {},
                 "detail": f"transform language {spec.get('language')!r} is not served"} for r in rows]
    namespace = {"__builtins__": _SAFE_BUILTINS, "math": math}
    try:
        exec(compile(spec.get("source", ""), "<transform>", "exec"), namespace)  # noqa: S102
        fn = namespace[spec.get("entrypoint", "transform")]
    except Exception as e:  # noqa: BLE001
        return [{"key": r.get("key"), "status": "unavailable", "outputs": {},
                 "detail": f"transform does not load: {e}"} for r in rows]
    answers = []
    for r in rows:
        try:
            out = fn(dict(r.get("features") or {}))
        except Exception as e:  # noqa: BLE001
            answers.append({"key": r.get("key"), "status": "abstained", "outputs": {},
                            "detail": f"transform raised {type(e).__name__}: {e}"})
            continue
        if not isinstance(out, dict):
            answers.append({"key": r.get("key"), "status": "abstained", "outputs": {},
                            "detail": f"transform returned {type(out).__name__}, not a dict of outputs"})
            continue
        answers.append({"key": r.get("key"), "status": "answered", "outputs": out, "detail": None})
    return answers
