"""Shared training engine: the framework-agnostic core every adapter builds on.

Responsibilities:
  * Resolve the learning objective (classification vs regression) from the model
    definition rather than silently coercing labels.
  * Produce a leakage-aware time-series train/val/test split with an embargo gap.
  * Standardize features using train-split statistics, persisted for inference.
  * Emit a consistent metrics scorecard across all adapters.
  * Wrap the framework-native artifact in a self-describing "bundle" so the
    inference side reconstructs the exact feature order + scaler used here.

The bundle format is intentionally trivial and dependency-free so the separate
inference service can parse it without sharing code:

    MAGIC(8) | header_len(u32 LE) | header_json(utf-8) | inner_artifact_bytes
"""

from __future__ import annotations

import json
import os
import random
import struct
from dataclasses import dataclass, field

import numpy as np
import pandas as pd

MAGIC = b"TBNDL001"
BUNDLE_SCHEMA = "tb-bundle-1"


# --------------------------------------------------------------------------- #
# Objective resolution
# --------------------------------------------------------------------------- #
def resolve_objective(definition: dict) -> str:
    """classification | regression, derived from the definition (hp override wins)."""
    hp = definition.get("hyperparameters", {}) or {}
    explicit = hp.get("objective")
    if explicit in ("classification", "regression"):
        return explicit
    # Forecasters predict a continuous forward return; everything else defaults
    # to predicting up/down direction.
    kind = definition.get("model_kind") or definition.get("kind")
    if kind == "forecaster":
        return "regression"
    return "classification"


def direction_threshold(definition: dict) -> float:
    """Return threshold separating up (1) from down (0) for classification."""
    hp = definition.get("hyperparameters", {}) or {}
    try:
        return float(hp.get("direction_threshold", 0.0))
    except (TypeError, ValueError):
        return 0.0


def resolve_seed(definition: dict) -> int:
    hp = definition.get("hyperparameters", {}) or {}
    try:
        return int(hp.get("seed", 42))
    except (TypeError, ValueError):
        return 42


def seed_everything(definition: dict) -> int:
    """Seed Python/NumPy (and torch if importable) for reproducible runs."""
    seed = resolve_seed(definition)
    random.seed(seed)
    np.random.seed(seed)
    os.environ.setdefault("PYTHONHASHSEED", str(seed))
    try:  # torch is optional at import time for non-torch adapters
        import torch

        torch.manual_seed(seed)
    except Exception:  # noqa: BLE001
        pass
    return seed


# --------------------------------------------------------------------------- #
# Time-series split with embargo
# --------------------------------------------------------------------------- #
def split_indices(
    n: int, val_frac: float = 0.15, test_frac: float = 0.15, embargo: int = 0
) -> tuple[slice, slice, slice]:
    """Ordinal train/val/test split.

    The last ``embargo`` rows of the train and validation segments are dropped so
    a row's forward-return label cannot peek into the following segment.
    """
    if n <= 0:
        empty = slice(0, 0)
        return empty, empty, empty

    test_n = int(round(n * test_frac))
    val_n = int(round(n * val_frac))
    train_n = n - val_n - test_n
    if train_n < 1:
        # Degenerate (tiny) datasets: keep at least one training row.
        val_n = max(0, (n - 1) // 3)
        test_n = max(0, (n - 1) // 3)
        train_n = n - val_n - test_n

    emb = max(0, int(embargo))
    train_end = max(1, train_n - emb)
    val_start = train_n
    val_end = max(val_start, val_start + val_n - emb)
    test_start = train_n + val_n
    return slice(0, train_end), slice(val_start, val_end), slice(test_start, n)


# --------------------------------------------------------------------------- #
# Standardization
# --------------------------------------------------------------------------- #
def fit_scaler(X: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    mean = X.mean(axis=0)
    std = X.std(axis=0)
    std = np.where(std < 1e-8, 1.0, std)  # guard constant columns
    return mean, std


def apply_scaler(X: np.ndarray, mean: np.ndarray, std: np.ndarray) -> np.ndarray:
    if X.size == 0:
        return X
    return (X - mean) / std


# --------------------------------------------------------------------------- #
# Prepared training data
# --------------------------------------------------------------------------- #
@dataclass
class Prepared:
    objective: str
    feature_order: list[str]
    scaler: dict  # {"mean": [...], "std": [...]}
    hp: dict
    seed: int
    X_tr: np.ndarray
    y_tr: np.ndarray
    X_val: np.ndarray
    y_val: np.ndarray
    X_te: np.ndarray
    y_te: np.ndarray
    extra: dict = field(default_factory=dict)


def prepare(
    definition: dict,
    df: pd.DataFrame,
    *,
    val_frac: float = 0.15,
    test_frac: float = 0.15,
) -> Prepared:
    """Resolve objective, split, and standardize a training frame."""
    objective = resolve_objective(definition)
    seed = resolve_seed(definition)

    label_col = "label" if "label" in df.columns else df.columns[-1]
    feat_df = df.drop(columns=[label_col]).select_dtypes(include=["number"])
    feature_order = list(feat_df.columns)

    X = feat_df.to_numpy(dtype=np.float64)
    y_cont = df[label_col].to_numpy(dtype=np.float64)

    if objective == "classification":
        thr = direction_threshold(definition)
        y = (y_cont > thr).astype(np.float64)
    else:
        y = y_cont

    n = len(X)
    embargo = int(definition.get("_embargo_bars", 0) or 0)
    tr, va, te = split_indices(n, val_frac, test_frac, embargo)

    mean, std = fit_scaler(X[tr]) if (tr.stop or 0) > 0 else fit_scaler(X)
    Xs = apply_scaler(X, mean, std).astype(np.float32)

    return Prepared(
        objective=objective,
        feature_order=feature_order,
        scaler={"mean": mean.tolist(), "std": std.tolist()},
        hp=definition.get("hyperparameters", {}) or {},
        seed=seed,
        X_tr=Xs[tr],
        y_tr=y[tr].astype(np.float32),
        X_val=Xs[va],
        y_val=y[va].astype(np.float32),
        X_te=Xs[te],
        y_te=y[te].astype(np.float32),
    )


# --------------------------------------------------------------------------- #
# Metrics
# --------------------------------------------------------------------------- #
def classification_metrics(prefix: str, y_true, prob) -> dict:
    y_true = np.asarray(y_true).astype(int).ravel()
    prob = np.asarray(prob, dtype=float).ravel()
    out: dict = {}
    if y_true.size == 0 or prob.size == 0:
        return out
    from sklearn.metrics import accuracy_score, log_loss, roc_auc_score

    pred = (prob >= 0.5).astype(int)
    out[f"{prefix}_accuracy"] = float(accuracy_score(y_true, pred))
    if len(np.unique(y_true)) >= 2:
        try:
            out[f"{prefix}_auc"] = float(roc_auc_score(y_true, prob))
        except Exception:  # noqa: BLE001
            pass
        try:
            out[f"{prefix}_logloss"] = float(
                log_loss(y_true, np.clip(prob, 1e-6, 1 - 1e-6), labels=[0, 1])
            )
        except Exception:  # noqa: BLE001
            pass
    return out


def regression_metrics(prefix: str, y_true, pred) -> dict:
    y_true = np.asarray(y_true, dtype=float).ravel()
    pred = np.asarray(pred, dtype=float).ravel()
    out: dict = {}
    if y_true.size == 0 or pred.size == 0:
        return out
    from sklearn.metrics import mean_absolute_error, mean_squared_error, r2_score

    mse = float(mean_squared_error(y_true, pred))
    out[f"{prefix}_mse"] = mse
    out[f"{prefix}_rmse"] = float(np.sqrt(mse))
    out[f"{prefix}_mae"] = float(mean_absolute_error(y_true, pred))
    try:
        out[f"{prefix}_r2"] = float(r2_score(y_true, pred))
    except Exception:  # noqa: BLE001
        pass
    # Did we at least get the direction right?
    out[f"{prefix}_directional_accuracy"] = float(
        np.mean((np.sign(pred) == np.sign(y_true)).astype(float))
    )
    return out


def assemble_metrics(
    objective: str,
    parts: list[dict],
    *,
    framework_version: str,
    n_train: int,
    n_val: int,
    n_test: int,
    extra: dict | None = None,
) -> dict:
    """Merge metric fragments and set the canonical keys the Rust scorecard reads."""
    m: dict = {}
    for p in parts:
        m.update(p)
    m["objective"] = objective
    m["n_train"] = int(n_train)
    m["n_val"] = int(n_val)
    m["n_test"] = int(n_test)
    m["framework_version"] = framework_version

    # Canonical top-level aliases (scorecard.rs / regression.rs key off these).
    if objective == "classification":
        if "val_accuracy" in m:
            m["accuracy"] = m["val_accuracy"]
    else:
        if "val_rmse" in m:
            m["rmse"] = m["val_rmse"]
        if "val_mae" in m:
            m["mae"] = m["val_mae"]
    if extra:
        m.update(extra)
    return m


# --------------------------------------------------------------------------- #
# Torch minibatch training helper (shared by torch + forecaster adapters)
# --------------------------------------------------------------------------- #
def train_torch_model(
    model,
    X_tr: np.ndarray,
    y_tr: np.ndarray,
    X_val: np.ndarray,
    y_val: np.ndarray,
    *,
    objective: str,
    epochs: int,
    lr: float,
    batch_size: int,
    emit_progress,
    weight_decay: float = 0.0,
):
    import torch
    import torch.nn as nn

    Xtr = torch.tensor(np.asarray(X_tr, dtype=np.float32))
    ytr = torch.tensor(np.asarray(y_tr, dtype=np.float32)).unsqueeze(1)
    has_val = len(X_val) > 0
    if has_val:
        Xval = torch.tensor(np.asarray(X_val, dtype=np.float32))
        yval = torch.tensor(np.asarray(y_val, dtype=np.float32)).unsqueeze(1)

    opt = torch.optim.Adam(model.parameters(), lr=lr, weight_decay=weight_decay)
    loss_fn = nn.BCEWithLogitsLoss() if objective == "classification" else nn.MSELoss()

    n = len(Xtr)
    bs = max(1, min(int(batch_size), n)) if n else 1
    train_loss = float("nan")
    val_loss = float("nan")

    for epoch in range(epochs):
        model.train()
        perm = torch.randperm(n)
        running = 0.0
        for i in range(0, n, bs):
            idx = perm[i : i + bs]
            opt.zero_grad()
            out = model(Xtr[idx])
            loss = loss_fn(out, ytr[idx])
            loss.backward()
            opt.step()
            running += float(loss.item()) * len(idx)
        train_loss = running / max(n, 1)

        if has_val:
            model.eval()
            with torch.no_grad():
                val_loss = float(loss_fn(model(Xval), yval).item())

        pct = ((epoch + 1) / max(epochs, 1)) * 100.0
        emit_progress(
            "fitting",
            pct,
            {"epoch": epoch + 1, "train_loss": train_loss, "val_loss": val_loss},
        )

    return model, {"final_train_loss": train_loss, "final_val_loss": val_loss}


# --------------------------------------------------------------------------- #
# Bundle envelope
# --------------------------------------------------------------------------- #
def wrap_bundle(
    inner: bytes,
    *,
    framework: str,
    objective: str,
    feature_order: list[str],
    scaler: dict | None,
    arch: str | None = None,
    extra: dict | None = None,
) -> bytes:
    """Wrap a framework-native artifact in the self-describing bundle envelope."""
    header = {
        "schema_version": BUNDLE_SCHEMA,
        "framework": framework,
        "objective": objective,
        "feature_order": list(feature_order),
        "scaler": scaler,
        "arch": arch,
        "extra": extra or {},
    }
    hb = json.dumps(header, separators=(",", ":")).encode("utf-8")
    return MAGIC + struct.pack("<I", len(hb)) + hb + inner
