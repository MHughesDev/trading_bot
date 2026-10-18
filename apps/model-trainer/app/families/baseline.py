"""Baseline family: mean, EWMA, ridge, HAR, Log-HAR, HARQ (N-2.2, MD-06, MD-09).

A candidate is never read without the thing it has to beat, so a candidate is
always evaluated beside every applicable baseline **on the same folds**.

Wire shape for the evaluation response (stable; the Rust side parses it)
------------------------------------------------------------------------
`metrics["fold_ids"]`: `[int, ...]` - the fold indices every row below was scored on.

`metrics["family_baselines"]`: a list, one row per baseline (the key `metrics["baselines"]` is NOT
touched: it remains scoring.py's `{naive, zero_shot}` dict):

    {
      "name": "har",                  # == variant
      "family": "baseline",
      "variant": "har",
      "status": "ok" | "refused",     # refused: a required input is missing (see "reason")
      "reason": null | "<text>",
      "folds": [0, 1, 2],             # fold indices scored; equals metrics["fold_ids"] when ok
      "scores": {...} | null,         # pooled over the folds' test rows, see below
      "per_fold": {"0": {...}},       # same keys as scores, per fold (string keys)
      "series": {"point": [..], "sigma": [..] | null},
                                      # aligned row-for-row with metrics["series"]["realized"]
                                      # (last SERIES_CAP rows); null where the row is in no fold's
                                      # test window. For a realized_vol target "sigma" is the
                                      # forecast volatility in label units (HAR/HARQ/Log-HAR
                                      # always carry it, so QLIKE is computable downstream).
      "contamination": null           # baselines are never contaminated; foundation rows carry
                                      # {"model_cutoff": "YYYY-MM-DD", "contaminated": bool, ...}
    }

`scores` keys by target kind (all lower-is-better except `r2_oos`, `accuracy`):
  * realized_vol          : `qlike`, `mse`, `rmse`, `mae`, `n`
  * forward_return / rank : `rmse`, `mae`, `r2_oos` (vs the training mean), `n`
  * 0/1 labels (direction, meta_label, triple_barrier sign): `brier`, `accuracy`, `rmse`, `n`

`metrics["candidate"]` carries the candidate's scores computed by the same function on the same
folds (`{"folds": [...], "scores": {...}}`), so a scorecard compares like with like.
A `refused` row means "not computable" and must block, never pass (ADR-N-05).

`metrics["series"]` (attached by worker.run_evaluation): `{"target_kind": "return"|"vol",
"realized": [...], "point": [...], "sigma": [...]|null}`, each capped at SERIES_CAP rows, most
recent kept. `metrics["candidate"]` also carries `series` in the same alignment.
"""

from __future__ import annotations

import pickle
from typing import Callable

import numpy as np
import pandas as pd

from .. import engine, failures

VOL_ONLY = ("har", "log_har", "harq")
ALL_BASELINES = ("mean", "ewma", "ridge", "har", "log_har", "harq")
_EPS = 1e-12
SERIES_CAP = 20000


class MissingRequirement(failures.DataError):
    """A baseline's required input column is absent (HARQ without realised quarticity)."""


class BaselineMissing(failures.TrainerFailure):
    """A candidate was trained or evaluated with no baseline beside it."""

    terminal = failures.DATA_ERROR


class UnsupportedTarget(failures.DataError):
    pass


# --------------------------------------------------------------------------- #
# Target + folds
# --------------------------------------------------------------------------- #
def target_kind(definition: dict) -> str:
    for src in (definition.get("target"), (definition.get("dataset") or {}).get("target"),
                definition.get("label_spec")):
        if isinstance(src, dict) and src.get("kind"):
            return str(src["kind"])
    return "forward_return"


def default_baselines(kind: str) -> list[str]:
    return list(ALL_BASELINES) if kind == "realized_vol" else ["mean", "ewma", "ridge"]


def _hp(definition: dict) -> dict:
    out = dict(definition.get("hyperparameters") or {})
    out.update(definition.get("baseline") or {})
    return out


def fold_slices(folds, n: int, definition: dict | None = None) -> list[tuple[int, slice, slice]]:
    """`(index, train, test)` per fold. No folds -> the engine's ordinal split, one fold."""
    if folds:
        out = []
        for f in folds:
            g = (lambda k: f[k]) if isinstance(f, dict) else (lambda k: getattr(f, k))
            out.append((int(g("index")), slice(g("train_start"), g("train_end")),
                        slice(g("test_start"), g("test_end"))))
        return out
    emb = int((definition or {}).get("_embargo_bars", 0) or 0)
    tr, _va, te = engine.split_indices(n, embargo=emb)
    return [(0, tr, te)]


# --------------------------------------------------------------------------- #
# Scores
# --------------------------------------------------------------------------- #
def qlike(y: np.ndarray, f: np.ndarray) -> float:
    """QLIKE = mean(y/f - log(y/f) - 1); y, f in the same (positive) unit."""
    y = np.maximum(np.asarray(y, float), _EPS)
    f = np.maximum(np.asarray(f, float), _EPS)
    r = y / f
    return float(np.mean(r - np.log(r) - 1.0))


def score_point(kind: str, y: np.ndarray, pred: np.ndarray, train_mean: float) -> dict:
    y = np.asarray(y, float)
    pred = np.asarray(pred, float)
    ok = np.isfinite(y) & np.isfinite(pred)
    y, pred = y[ok], pred[ok]
    n = int(len(y))
    if n == 0:
        return {"n": 0}
    err = y - pred
    out: dict = {"n": n, "rmse": float(np.sqrt(np.mean(err**2))), "mae": float(np.mean(np.abs(err)))}
    if kind == "realized_vol":
        out["qlike"] = qlike(y, pred)
        out["mse"] = float(np.mean(err**2))
    elif set(np.unique(y)).issubset({0.0, 1.0}):
        out["brier"] = float(np.mean(err**2))
        out["accuracy"] = float(np.mean((pred > 0.5) == (y > 0.5)))
    else:
        denom = float(np.sum((y - train_mean) ** 2))
        out["r2_oos"] = float(1.0 - np.sum(err**2) / denom) if denom > 0 else float("nan")
    return out


# --------------------------------------------------------------------------- #
# HAR design matrices (causal: rolling windows over the past rows only)
# --------------------------------------------------------------------------- #
def _require(df: pd.DataFrame, col: str, variant: str) -> np.ndarray:
    if col not in df.columns:
        raise MissingRequirement(f"baseline {variant!r} requires column {col!r} in the dataset")
    return df[col].to_numpy(dtype=float)


def har_design(df: pd.DataFrame, variant: str, windows=(1, 5, 22)) -> np.ndarray:
    rv = _require(df, "realized_variance", variant)
    cols = [pd.Series(rv).rolling(int(w), min_periods=1).mean().to_numpy() for w in windows]
    if variant == "log_har":
        cols = [np.log(np.maximum(c, _EPS)) for c in cols]
    elif variant == "harq":
        rq = _require(df, "realized_quarticity", variant)
        cols.append(np.sqrt(np.maximum(rq, 0.0)) * cols[0])
    return np.column_stack(cols)


def _ols(X: np.ndarray, y: np.ndarray) -> np.ndarray:
    A = np.column_stack([np.ones(len(X)), X])
    beta, *_ = np.linalg.lstsq(A, y, rcond=None)
    return beta


def _lin(beta: np.ndarray, X: np.ndarray) -> np.ndarray:
    return beta[0] + X @ beta[1:]


# --------------------------------------------------------------------------- #
# fit / predict by variant. State is a small picklable dict.
# --------------------------------------------------------------------------- #
def fit_state(variant: str, df: pd.DataFrame, train: slice, definition: dict) -> dict:
    """Fit on `df.iloc[train]` only."""
    hp = _hp(definition)
    label_col = "label" if "label" in df.columns else df.columns[-1]
    kind = target_kind(definition)
    y_all = df[label_col].to_numpy(dtype=float)
    ytr = y_all[train]
    ytr_f = ytr[np.isfinite(ytr)]
    if len(ytr_f) < 3:
        raise failures.DataError(f"baseline {variant!r}: fewer than 3 finite training labels")
    if variant in VOL_ONLY and kind != "realized_vol":
        raise UnsupportedTarget(f"baseline {variant!r} is a volatility baseline; target is {kind!r}")
    st: dict = {"variant": variant, "train_mean": float(ytr_f.mean())}
    if variant == "mean":
        st["value"] = st["train_mean"]
    elif variant == "ewma":
        hl = float(hp.get("ewma_halflife", 20.0))
        st["value"] = float(pd.Series(ytr_f).ewm(halflife=hl, adjust=True).mean().iloc[-1])
        st["halflife"] = hl
    elif variant == "ridge":
        from sklearn.linear_model import Ridge

        feats = engine.feature_frame(df, label_col)
        X = feats.to_numpy(dtype=float)
        Xtr = X[train]
        ok = np.isfinite(ytr) & np.all(np.isfinite(Xtr), axis=1)
        if ok.any():
            mu, sd = engine.fit_scaler(Xtr[ok])
        else:
            mu, sd = np.zeros(X.shape[1]), np.ones(X.shape[1])
        m = Ridge(alpha=float(hp.get("ridge_alpha", 1.0))).fit(
            engine.apply_scaler(Xtr[ok], mu, sd), ytr[ok]
        )
        st.update(model=m, mean=mu, std=sd, feature_order=list(feats.columns))
    elif variant in VOL_ONLY:
        windows = tuple(hp.get("har_windows", (1, 5, 22)))
        D = har_design(df, variant, windows)
        ok = np.isfinite(y_all) & np.all(np.isfinite(D), axis=1)
        sel = np.zeros(len(df), bool)
        sel[train] = True
        ok &= sel
        if variant == "log_har":
            ok &= y_all > 0
        if ok.sum() < D.shape[1] + 2:
            raise failures.DataError(f"baseline {variant!r}: too few training rows ({int(ok.sum())})")
        yt = np.log(np.maximum(y_all[ok], _EPS)) if variant == "log_har" else y_all[ok]
        beta = _ols(D[ok], yt)
        st.update(beta=beta, windows=list(windows))
        if variant == "log_har":
            st["resid_var"] = float(np.var(yt - _lin(beta, D[ok])))
    else:
        raise failures.DataError(f"unknown baseline {variant!r}")
    return st


def predict_state(st: dict, df: pd.DataFrame) -> np.ndarray:
    """Predictions for every row of `df` (rolling features see only rows <= t)."""
    v = st["variant"]
    n = len(df)
    if v in ("mean", "ewma"):
        return np.full(n, st["value"], dtype=float)
    if v == "ridge":
        feats = df[st["feature_order"]].to_numpy(dtype=float)
        X = np.nan_to_num(engine.apply_scaler(feats, st["mean"], st["std"]))
        return st["model"].predict(X)
    D = har_design(df, v, tuple(st["windows"]))
    lin = _lin(st["beta"], D)
    if v == "log_har":
        return np.exp(lin + 0.5 * st["resid_var"])
    return np.maximum(lin, _EPS)


def fit_predict_fold(variant: str, df: pd.DataFrame, train: slice, test: slice, definition: dict):
    st = fit_state(variant, df, train, definition)
    pred = predict_state(st, df)[test]
    return st, pred


# --------------------------------------------------------------------------- #
# Evaluation beside a candidate, on identical folds
# --------------------------------------------------------------------------- #
def _pool(kind, ys, ps, means):
    y = np.concatenate(ys)
    p = np.concatenate(ps)
    return score_point(kind, y, p, float(np.mean(means)) if means else 0.0)


def score_candidate(definition: dict, df: pd.DataFrame, pred: np.ndarray, folds=None) -> dict:
    """The candidate's point predictions scored exactly as the baselines are."""
    kind = target_kind(definition)
    label_col = "label" if "label" in df.columns else df.columns[-1]
    y = df[label_col].to_numpy(dtype=float)
    pred = np.asarray(pred, float)
    fs = fold_slices(folds, len(df), definition)
    ys, ps, means, per = [], [], [], {}
    for idx, tr, te in fs:
        ytr = y[tr]
        ytr = ytr[np.isfinite(ytr)]
        m = float(ytr.mean()) if len(ytr) else 0.0
        means.append(m)
        ys.append(y[te])
        ps.append(pred[te])
        per[str(idx)] = score_point(kind, y[te], pred[te], m)
    full = np.full(len(df), np.nan)
    for _i, _tr, te in fs:
        full[te] = pred[te]
    pts = series_list(full)
    return {"folds": [i for i, _, _ in fs], "scores": _pool(kind, ys, ps, means), "per_fold": per,
            "series": {"point": pts, "sigma": pts if kind == "realized_vol" else None}}


def series_list(arr: np.ndarray, cap: int = SERIES_CAP) -> list:
    """Most recent `cap` values as JSON-safe floats (NaN -> None)."""
    a = np.asarray(arr, float)[-cap:]
    return [None if not np.isfinite(v) else float(v) for v in a]


def evaluate_baselines(
    definition: dict,
    df: pd.DataFrame,
    folds=None,
    names: list[str] | None = None,
) -> tuple[list[dict], list[int]]:
    """Fit and score each baseline on the same folds. Returns `(rows, fold_ids)`.

    A baseline whose required input is absent yields a `refused` row (never a
    fabricated number). An explicit empty `names` is refused outright.
    """
    if names is not None and len(names) == 0:
        raise BaselineMissing("a candidate must be evaluated beside at least one baseline")
    kind = target_kind(definition)
    names = list(names) if names is not None else default_baselines(kind)
    label_col = "label" if "label" in df.columns else df.columns[-1]
    y = df[label_col].to_numpy(dtype=float)
    fs = fold_slices(folds, len(df), definition)
    fold_ids = [i for i, _, _ in fs]
    rows = []
    for name in names:
        row = {"name": name, "family": "baseline", "variant": name, "status": "ok", "reason": None,
               "folds": fold_ids, "scores": None, "per_fold": {}, "contamination": None,
               "series": None}
        ys, ps, means = [], [], []
        full = np.full(len(df), np.nan)
        try:
            for idx, tr, te in fs:
                st, pred = fit_predict_fold(name, df, tr, te, definition)
                ys.append(y[te])
                ps.append(pred)
                full[te] = pred
                means.append(st["train_mean"])
                row["per_fold"][str(idx)] = score_point(kind, y[te], pred, st["train_mean"])
            row["scores"] = _pool(kind, ys, ps, means)
            pts = series_list(full)
            row["series"] = {"point": pts, "sigma": pts if kind == "realized_vol" else None}
        except (MissingRequirement, UnsupportedTarget) as e:
            row.update(status="refused", reason=str(e), folds=[], scores=None, per_fold={})
        rows.append(row)
    wanted = _hp(definition).get("foundation")
    if wanted:  # opt-in: definition["baseline"]["foundation"] = ["chronos", ...]
        from . import foundation

        rows.extend(foundation.rows(definition, df, folds, list(wanted)))
    return rows, fold_ids


def train_with_baselines(
    definition: dict,
    df: pd.DataFrame,
    candidate_train: Callable,
    emit: Callable,
    folds=None,
    baselines: list[str] | None = None,
) -> tuple[bytes, dict]:
    """Train a candidate and its baselines on the same folds.

    Refuses (`BaselineMissing`) when `baselines` is an explicit empty list: a
    candidate with nothing to beat is not a result.
    """
    if baselines is not None and len(baselines) == 0:
        raise BaselineMissing("refusing to train a candidate with no baseline beside it")
    art, metrics = candidate_train(definition, df, emit)
    rows, fold_ids = evaluate_baselines(definition, df, folds, baselines)
    metrics = {**(metrics or {}), "family_baselines": rows, "fold_ids": fold_ids}
    return art, metrics


# --------------------------------------------------------------------------- #
# Registry entry points: train a baseline as a standalone model
# --------------------------------------------------------------------------- #
def _train(variant: str, definition: dict, df: pd.DataFrame, emit) -> tuple[bytes, dict]:
    engine.seed_everything(definition)
    wf = definition.get("_wf_fold")
    if wf:
        train, test = slice(wf["train_start"], wf["train_end"]), slice(wf["test_start"], wf["test_end"])
    else:
        tr, _va, te = engine.split_indices(len(df), embargo=int(definition.get("_embargo_bars", 0) or 0))
        train, test = tr, te
    emit("fitting", 20.0, {"baseline": variant})
    st, pred = fit_predict_fold(variant, df, train, test, definition)
    kind = target_kind(definition)
    label_col = "label" if "label" in df.columns else df.columns[-1]
    scores = score_point(kind, df[label_col].to_numpy(dtype=float)[test], pred, st["train_mean"])
    extra = {"baseline": variant, "test_scores": scores}
    extra.update({f"test_{k}": v for k, v in scores.items() if k != "n"})
    metrics = engine.assemble_metrics(
        "regression", [], framework_version="baseline-1", n_train=len(df.iloc[train]), n_val=0,
        n_test=len(pred), extra=extra,
    )
    inner = pickle.dumps(dict(st))
    bundle = engine.wrap_bundle(
        inner, framework="baseline", objective="regression",
        feature_order=st.get("feature_order") or [], scaler=None, output_kind="point",
        extra={"family": "baseline", "variant": variant, "target": kind},
    )
    emit("fitting", 100.0, metrics)
    return bundle, metrics


def train_mean(definition, df, emit):
    return _train("mean", definition, df, emit)


def train_ewma(definition, df, emit):
    return _train("ewma", definition, df, emit)


def train_ridge(definition, df, emit):
    return _train("ridge", definition, df, emit)


def train_har(definition, df, emit):
    return _train("har", definition, df, emit)


def train_log_har(definition, df, emit):
    return _train("log_har", definition, df, emit)


def train_harq(definition, df, emit):
    return _train("harq", definition, df, emit)
