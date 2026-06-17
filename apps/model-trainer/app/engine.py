"""Shared training engine: the framework-agnostic core every adapter builds on.

Responsibilities:
  * Resolve the learning objective (classification vs regression) from the model
    definition rather than silently coercing labels.
  * Produce a leakage-aware time-series train/val/test split with an embargo gap.
  * Standardize features using train-split statistics, persisted for inference.
  * Fit a σ scaler on train targets (no leakage) for devolatization (I-1.7).
  * Repair quantile-crossing via sort after prediction (I-1.8).
  * Validate the distributional output contract before writing the bundle (I-1.12).
  * Emit a consistent metrics scorecard across all adapters.
  * Wrap the framework-native artifact in a self-describing "bundle" so the
    inference side reconstructs the exact feature order + scaler used here.

Bundle format (tb-bundle-1):

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
    kind = definition.get("model_kind") or definition.get("kind")
    if kind == "forecaster":
        return "regression"
    return "classification"


def direction_threshold(definition: dict) -> float:
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
    seed = resolve_seed(definition)
    random.seed(seed)
    np.random.seed(seed)
    os.environ.setdefault("PYTHONHASHSEED", str(seed))
    try:
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
    """Ordinal train/val/test split with embargo gap."""
    if n <= 0:
        empty = slice(0, 0)
        return empty, empty, empty

    test_n = int(round(n * test_frac))
    val_n = int(round(n * val_frac))
    train_n = n - val_n - test_n
    if train_n < 1:
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
# Feature standardization
# --------------------------------------------------------------------------- #
def fit_scaler(X: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    mean = X.mean(axis=0)
    std = X.std(axis=0)
    std = np.where(std < 1e-8, 1.0, std)
    return mean, std


def apply_scaler(X: np.ndarray, mean: np.ndarray, std: np.ndarray) -> np.ndarray:
    if X.size == 0:
        return X
    return (X - mean) / std


# --------------------------------------------------------------------------- #
# σ scaler for devolatization (I-1.7)
# --------------------------------------------------------------------------- #
def fit_sigma_scaler(y_train: np.ndarray) -> float:
    """Fit a σ scaler from training targets.

    σ = std(y_train), clamped to >= 1e-8 so division is safe.
    Fit on training data only — no leakage into val/test.
    """
    if y_train.size == 0:
        return 1.0
    sigma = float(np.std(y_train))
    return max(sigma, 1e-8)


def devol_targets(y: np.ndarray, sigma: float) -> np.ndarray:
    """Standardize target values by σ (devolatization). Models predict in σ-units."""
    if sigma <= 0:
        return y
    return y / sigma


def rescale_quantiles(q_sigma: np.ndarray, sigma: float) -> np.ndarray:
    """Rescale σ-unit quantiles back to return units."""
    return q_sigma * sigma


# --------------------------------------------------------------------------- #
# Quantile-crossing repair (I-1.8)
# --------------------------------------------------------------------------- #
def repair_quantiles(q: np.ndarray) -> tuple[np.ndarray, int]:
    """Enforce monotonicity in a quantile array via sorting.

    Args:
        q: shape (n_instances, n_quantiles) or (n_quantiles,).

    Returns:
        (repaired_q, n_repairs) where n_repairs counts instances that needed repair.
    """
    q = np.atleast_2d(q)
    n_repairs = 0
    repaired = np.empty_like(q)
    for i, row in enumerate(q):
        sorted_row = np.sort(row)
        if not np.array_equal(row, sorted_row):
            n_repairs += 1
        repaired[i] = sorted_row
    return repaired.squeeze(0) if repaired.shape[0] == 1 else repaired, n_repairs


# --------------------------------------------------------------------------- #
# Distribution contract validation (I-1.12)
# --------------------------------------------------------------------------- #
def validate_distribution(
    quantiles: np.ndarray,
    levels: list[float],
    sigma: float,
) -> None:
    """Raise ValueError if the distributional output contract is violated.

    Called at train completion before writing the artifact to the registry
    (Python side of I-1.12). Rust validates again at serve time.
    """
    n = len(levels)
    if n == 0:
        raise ValueError("quantile_levels must not be empty")
    for i, l in enumerate(levels):
        if not (0.0 < l < 1.0):
            raise ValueError(f"level[{i}]={l} not in (0,1)")
        if i > 0 and l <= levels[i - 1]:
            raise ValueError(f"quantile_levels not strictly increasing at index {i}")
    if sigma <= 0:
        raise ValueError(f"sigma={sigma} must be > 0")
    q = np.atleast_2d(quantiles)
    if q.shape[-1] != n:
        raise ValueError(
            f"quantiles last dim {q.shape[-1]} != levels count {n}"
        )
    for i, row in enumerate(q):
        if not np.all(np.isfinite(row)):
            raise ValueError(f"non-finite quantile at instance {i}")
        for j in range(1, len(row)):
            if row[j] < row[j - 1]:
                raise ValueError(
                    f"quantiles not monotone at instance {i}, position {j}"
                )


# --------------------------------------------------------------------------- #
# Pinball loss / CRPS (used in HPO and metrics)
# --------------------------------------------------------------------------- #
def pinball_loss(y_true: np.ndarray, q_preds: np.ndarray, levels: list[float]) -> float:
    """Mean pinball loss across all quantile levels (= mean CRPS for quantile regression)."""
    y = np.asarray(y_true, dtype=float).ravel()
    if len(y) == 0 or len(levels) == 0:
        return float("nan")
    total = 0.0
    for i, alpha in enumerate(levels):
        q = np.asarray(q_preds)[:, i] if q_preds.ndim == 2 else np.asarray(q_preds)
        err = y - q
        total += float(np.mean(np.where(err >= 0, alpha * err, (alpha - 1.0) * err)))
    return total / len(levels)


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
    # σ of training targets; 1.0 when no devolatization was applied.
    sigma: float = 1.0
    # quantile grid from definition["output"]["quantile_levels"]; empty for point models.
    quantile_levels: list = field(default_factory=list)
    extra: dict = field(default_factory=dict)


def _extract_quantile_levels(definition: dict) -> list[float]:
    output = (definition.get("output") or {})
    return list(output.get("quantile_levels") or [])


def _apply_label_transforms(
    y_cont: np.ndarray,
    tr: slice,
    definition: dict,
) -> tuple[np.ndarray, float]:
    """Apply devolatization when label_spec.devol is true.

    Fits σ on train only; returns (transformed_y, sigma).
    """
    label_spec = definition.get("label_spec") or {}
    devol = bool(label_spec.get("devol", False))
    sigma = fit_sigma_scaler(y_cont[tr]) if devol else 1.0
    y = devol_targets(y_cont, sigma) if devol else y_cont
    return y, sigma


def prepare(
    definition: dict,
    df: pd.DataFrame,
    *,
    val_frac: float = 0.15,
    test_frac: float = 0.15,
) -> Prepared:
    """Resolve objective, split, and standardize a training frame."""
    wf_fold_dict = definition.get("_wf_fold")
    if wf_fold_dict is not None:
        from .schemas import FoldSpec  # local import avoids circularity
        return prepare_with_fold(definition, df, FoldSpec(**wf_fold_dict))

    objective = resolve_objective(definition)
    seed = resolve_seed(definition)

    label_col = "label" if "label" in df.columns else df.columns[-1]
    feat_df = df.drop(columns=[label_col]).select_dtypes(include=["number"])
    feature_order = list(feat_df.columns)

    X = feat_df.to_numpy(dtype=np.float64)
    y_cont = df[label_col].to_numpy(dtype=np.float64)

    n = len(X)
    embargo = int(definition.get("_embargo_bars", 0) or 0)
    tr, va, te = split_indices(n, val_frac, test_frac, embargo)

    y, sigma = _apply_label_transforms(y_cont, tr, definition)

    if objective == "classification":
        thr = direction_threshold(definition)
        y_final = (y > thr).astype(np.float64)
    else:
        y_final = y

    mean, std = fit_scaler(X[tr]) if (tr.stop or 0) > 0 else fit_scaler(X)
    Xs = apply_scaler(X, mean, std).astype(np.float32)

    return Prepared(
        objective=objective,
        feature_order=feature_order,
        scaler={"mean": mean.tolist(), "std": std.tolist()},
        hp=definition.get("hyperparameters", {}) or {},
        seed=seed,
        X_tr=Xs[tr],
        y_tr=y_final[tr].astype(np.float32),
        X_val=Xs[va],
        y_val=y_final[va].astype(np.float32),
        X_te=Xs[te],
        y_te=y_final[te].astype(np.float32),
        sigma=sigma,
        quantile_levels=_extract_quantile_levels(definition),
    )


def prepare_with_fold(
    definition: dict,
    df: pd.DataFrame,
    fold,  # FoldSpec from schemas.py
) -> Prepared:
    """Prepare training data using Rust-computed fold index ranges (I-0.10)."""
    objective = resolve_objective(definition)
    seed = resolve_seed(definition)

    label_col = "label" if "label" in df.columns else df.columns[-1]
    feat_df = df.drop(columns=[label_col]).select_dtypes(include=["number"])
    feature_order = list(feat_df.columns)

    X = feat_df.to_numpy(dtype=np.float64)
    y_cont = df[label_col].to_numpy(dtype=np.float64)

    tr_sl = slice(fold.train_start, fold.train_end)
    va_sl = slice(fold.cal_start, fold.cal_end)
    te_sl = slice(fold.test_start, fold.test_end)

    y, sigma = _apply_label_transforms(y_cont, tr_sl, definition)

    if objective == "classification":
        thr = direction_threshold(definition)
        y_final = (y > thr).astype(np.float64)
    else:
        y_final = y

    mean, std = fit_scaler(X[tr_sl]) if fold.train_end > fold.train_start else fit_scaler(X)
    Xs = apply_scaler(X, mean, std).astype(np.float32)

    return Prepared(
        objective=objective,
        feature_order=feature_order,
        scaler={"mean": mean.tolist(), "std": std.tolist()},
        hp=definition.get("hyperparameters", {}) or {},
        seed=seed,
        X_tr=Xs[tr_sl],
        y_tr=y_final[tr_sl].astype(np.float32),
        X_val=Xs[va_sl],
        y_val=y_final[va_sl].astype(np.float32),
        X_te=Xs[te_sl],
        y_te=y_final[te_sl].astype(np.float32),
        sigma=sigma,
        quantile_levels=_extract_quantile_levels(definition),
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
    out[f"{prefix}_directional_accuracy"] = float(
        np.mean((np.sign(pred) == np.sign(y_true)).astype(float))
    )
    return out


def quantile_metrics(prefix: str, y_true, q_preds, levels: list[float]) -> dict:
    """CRPS (mean pinball) and per-level coverage for a quantile forecast."""
    y_true = np.asarray(y_true, dtype=float).ravel()
    q_preds = np.atleast_2d(np.asarray(q_preds, dtype=float))
    out: dict = {}
    if y_true.size == 0 or q_preds.size == 0 or not levels:
        return out
    crps = pinball_loss(y_true, q_preds, levels)
    out[f"{prefix}_crps"] = crps
    # Per-level empirical coverage
    for i, alpha in enumerate(levels):
        q = q_preds[:, i]
        coverage = float(np.mean(y_true <= q))
        out[f"{prefix}_coverage_{int(alpha * 100):02d}"] = coverage
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
    m: dict = {}
    for p in parts:
        m.update(p)
    m["objective"] = objective
    m["n_train"] = int(n_train)
    m["n_val"] = int(n_val)
    m["n_test"] = int(n_test)
    m["framework_version"] = framework_version

    if objective == "classification":
        if "val_accuracy" in m:
            m["accuracy"] = m["val_accuracy"]
    else:
        if "val_rmse" in m:
            m["rmse"] = m["val_rmse"]
        if "val_mae" in m:
            m["mae"] = m["val_mae"]
        if "val_crps" in m:
            m["crps"] = m["val_crps"]
    if extra:
        m.update(extra)
    return m


# --------------------------------------------------------------------------- #
# Torch minibatch training helper
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
# Bundle envelope (I-1.10)
# --------------------------------------------------------------------------- #
def wrap_bundle(
    inner: bytes,
    *,
    framework: str,
    objective: str,
    feature_order: list[str],
    scaler: dict | None,
    arch: str | None = None,
    output_kind: str = "point",
    quantile_levels: list[float] | None = None,
    sigma_scaler: float | None = None,
    calibration: dict | None = None,
    extra: dict | None = None,
) -> bytes:
    """Wrap a framework-native artifact in the self-describing bundle envelope.

    New in I-1.10: `output_kind`, `quantile_levels`, `sigma_scaler`, `calibration`
    are carried for distributional models; legacy point bundles (no new fields)
    still load unchanged.
    """
    header = {
        "schema_version": BUNDLE_SCHEMA,
        "framework": framework,
        "objective": objective,
        "feature_order": list(feature_order),
        "scaler": scaler,
        "arch": arch,
        "output_kind": output_kind,
        "quantile_levels": quantile_levels,
        "sigma_scaler": sigma_scaler,
        "calibration": calibration,
        "extra": extra or {},
    }
    hb = json.dumps(header, separators=(",", ":")).encode("utf-8")
    return MAGIC + struct.pack("<I", len(hb)) + hb + inner
